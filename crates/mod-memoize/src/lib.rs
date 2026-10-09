use config::epoch::{get_current_epoch, ConfigEpoch};
use config::{any_err, from_lua_value, get_or_create_module, load_config, serialize_options};
use dashmap::DashMap;
use kumo_prometheus::declare_metric;
use kumo_prometheus::prometheus::Counter;
use lruttl::{ItemLookup, LruCacheWithTtl};
use mlua::{
    FromLua, Function, IntoLua, Lua, LuaSerdeExt, MetaMethod, MultiValue, UserData,
    UserDataMethods, UserDataRef,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Memoized is a helper type that allows native Rust types to be captured
/// in memoization caches.
/// Unfortunately, we cannot automatically make that work for all UserData
/// that are exported to lua, but we can make it simple for a type to opt-in
/// to that behavior.
///
/// When you impl UserData for your type, you can call
/// `Memoized::impl_memoize(methods)` from your add_methods impl.
/// That will add a metamethod to your UserData type that will clone your
/// value and wrap it into a Memoized wrapper.
///
/// Since Clone is used, it is recommended that you use an Arc inside your
/// type to avoid making large or expensive clones.
#[derive(Clone, mlua::FromLua)]
pub struct Memoized {
    pub to_value: Arc<dyn Fn(&Lua) -> mlua::Result<mlua::Value> + Send + Sync>,
}

impl PartialEq for Memoized {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.to_value, &other.to_value)
    }
}

impl Memoized {
    /// Call this from your `UserData::add_methods` implementation to
    /// enable memoization for your UserData type
    pub fn impl_memoize<T, M>(methods: &mut M)
    where
        T: UserData + Send + Sync + Clone + 'static,
        M: UserDataMethods<T>,
    {
        methods.add_meta_method(
            "__memoize",
            move |_lua, this, _: ()| -> mlua::Result<Memoized> {
                let this = this.clone();
                Ok(Memoized {
                    to_value: Arc::new(move |lua| this.clone().into_lua(lua)),
                })
            },
        );
    }
}

impl UserData for Memoized {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoizeParams {
    #[serde(with = "duration_serde")]
    pub ttl: Duration,
    pub capacity: usize,
    pub name: String,
    #[serde(default)]
    pub invalidate_with_epoch: bool,
    #[serde(default)]
    pub retry_on_populate_timeout: bool,
    #[serde(default, with = "duration_serde")]
    pub populate_timeout: Option<Duration>,
    #[serde(default)]
    pub allow_stale_reads: bool,
    #[serde(default)]
    pub detached: Option<bool>,
}

#[derive(Clone, Hash, Eq, PartialEq)]
pub enum MapKey {
    Integer(mlua::Integer),
    String(Vec<u8>),
}

impl MapKey {
    pub fn from_lua(v: mlua::Value) -> Option<Self> {
        match v {
            mlua::Value::String(s) => Some(Self::String(s.as_bytes().to_vec())),
            mlua::Value::Integer(n) => Some(Self::Integer(n)),
            _ => None,
        }
    }

    pub fn as_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        match self {
            Self::Integer(j) => Ok(mlua::Value::Integer(j)),
            Self::String(b) => Ok(mlua::Value::String(lua.create_string(b)?)),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum CacheValue {
    Table(Arc<HashMap<MapKey, CacheValue>>),
    Json(serde_json::Value),
    Memoized(Memoized),
}

impl std::fmt::Debug for CacheValue {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("CacheValue").finish()
    }
}

impl FromLua for CacheValue {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        match value {
            mlua::Value::UserData(ud) => {
                let mt = ud.metatable()?;
                let func: Function = mt.get("__memoize")?;
                let m: Memoized = func.call(mlua::Value::UserData(ud))?;
                Ok(Self::Memoized(m))
            }
            mlua::Value::Table(tbl) => {
                let mut map = HashMap::new();
                for pair in tbl.pairs::<mlua::Value, mlua::Value>() {
                    let (key, value) = pair?;
                    let key = match key {
                        mlua::Value::Integer(n) => MapKey::Integer(n),
                        mlua::Value::String(n) => MapKey::String(n.as_bytes().to_vec()),
                        _ => {
                            return Err(anyhow::anyhow!(
                                "table key {key:?} cannot be used as a key in a memoizable table"
                            ))
                            .map_err(any_err)
                        }
                    };
                    let value = CacheValue::from_lua(value, lua)?;
                    map.insert(key, value);
                }
                Ok(Self::Table(map.into()))
            }
            _ => Ok(Self::Json(from_lua_value(lua, value)?)),
        }
    }
}

impl IntoLua for CacheValue {
    fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        self.as_lua(lua)
    }
}

impl CacheValue {
    pub fn as_lua(&self, lua: &Lua) -> mlua::Result<mlua::Value> {
        match self {
            Self::Json(j) => lua.to_value_with(j, serialize_options()),
            Self::Memoized(m) => (m.to_value)(lua),
            Self::Table(m) => Ok(mlua::Value::UserData(
                lua.create_userdata(MemoizedTable::Shared(m.clone()))?,
            )),
        }
    }
}

/// MemoizedTable is a helper type that is returned to represent
/// cached table values.  We'll return the Shared variant by
/// default as that presents the cheapest way to return the cached
/// data--only a clone of the underlying Arc is required to return
/// the value.
///
/// This type implements __index, __newindex, __len, and __pairs
/// metamethods which allow reading and iterating the table.
///
/// Writing to the table via __newindex will "unshare" the table in
/// a similar manner to the Cow type, creating a mutable copy of the top
/// level of the table.
enum MemoizedTable {
    Shared(Arc<HashMap<MapKey, CacheValue>>),
    Mut(HashMap<MapKey, CacheValue>),
}

impl MemoizedTable {
    /// Get a reference to the table, facilitating get() and iter(),
    /// regardless of whether we are Shared or Mut.
    fn table(&self) -> &HashMap<MapKey, CacheValue> {
        match self {
            Self::Shared(s) => s,
            Self::Mut(s) => s,
        }
    }

    /// Transform Shared -> Mut
    fn unshare(&mut self) -> &mut HashMap<MapKey, CacheValue> {
        if let Self::Shared(t) = self {
            *self = Self::Mut(t.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        }

        match self {
            Self::Shared(_) => unreachable!(),
            Self::Mut(map) => map,
        }
    }
}

impl UserData for MemoizedTable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Index allows reading fields of the table
        methods.add_meta_method(MetaMethod::Index, move |lua, this, key: mlua::Value| {
            match MapKey::from_lua(key) {
                Some(key) => match this.table().get(&key) {
                    Some(value) => value.as_lua(lua),
                    None => Ok(mlua::Value::Nil),
                },
                None => Ok(mlua::Value::Nil),
            }
        });

        // NewIndex allows writing fields of the table
        methods.add_meta_method_mut(
            MetaMethod::NewIndex,
            move |lua, this, (key, value): (mlua::Value, mlua::Value)| match MapKey::from_lua(key) {
                Some(key) => {
                    let value = CacheValue::from_lua(value, lua)?;
                    this.unshare().insert(key, value);
                    Ok(())
                }
                None => Err(mlua::Error::external(
                    "invalid key type while trying to call __newindex and assign a value",
                )),
            },
        );
        methods.add_meta_method(MetaMethod::Len, move |_lua, this, ()| {
            Ok(this.table().len())
        });

        // Pairs iterates the keys of the table.
        // We use add_meta_function rather than add_meta_method here
        // because we need to return `this` as the "state" parameter
        // for use in a generic-for statement
        methods.add_meta_function(MetaMethod::Pairs, move |lua, this: mlua::Value| {
            // Maintain our own local idea of the control variable,
            // as it is much cheaper and simpler to iterate based
            // on skipping than to keep comparing keys
            let mut idx = 0;

            let iter_func =
                lua.create_function_mut(
                    move |lua, (state, _control): (UserDataRef<MemoizedTable>, mlua::Value)| {
                        match state.table().iter().nth(idx) {
                            Some((key, value)) => {
                                idx += 1;
                                let key = key.clone().as_lua(lua)?;
                                let value = value.as_lua(lua)?;
                                Ok((key, value))
                            }
                            None => Ok((mlua::Value::Nil, mlua::Value::Nil)),
                        }
                    },
                )?;

            // Return the iterator, state and control values.
            // The state and control will be passed back into iter_func
            // as the for-loop iterates.
            // Control is Nil here because we track our own idx
            // value in the iter_func closure.
            Ok((mlua::Value::Function(iter_func), this, mlua::Value::Nil))
        });
    }
}

#[derive(Clone, Debug)]
enum CacheEntry {
    Null,
    Single(CacheValue),
    Multi(Vec<CacheValue>),
}

impl CacheEntry {
    fn to_value(&self, lua: &Lua) -> mlua::Result<mlua::Value> {
        match self {
            Self::Null => Ok(mlua::Value::Nil),
            Self::Single(value) => value.as_lua(lua),
            Self::Multi(values) => {
                let mut result = vec![];
                for v in values {
                    result.push(v.as_lua(lua)?);
                }
                result.into_lua(lua)
            }
        }
    }

    fn from_multi_value(lua: &Lua, multi: MultiValue) -> mlua::Result<Self> {
        let mut values = multi.into_vec();
        if values.is_empty() {
            Ok(Self::Null)
        } else if values.len() == 1 {
            Ok(Self::Single(CacheValue::from_lua(
                values.pop().unwrap(),
                lua,
            )?))
        } else {
            let mut cvalues = vec![];
            for v in values.into_iter() {
                cvalues.push(CacheValue::from_lua(v, lua)?);
            }
            Ok(Self::Multi(cvalues))
        }
    }
}

struct MemoizeCache {
    params: MemoizeParams,
    cache: Arc<LruCacheWithTtl<CacheKey, CacheEntry>>,
}

static CACHES: LazyLock<DashMap<String, MemoizeCache>> = LazyLock::new(DashMap::new);

type CacheKey = (Option<ConfigEpoch>, String);

fn get_cache_by_name(
    name: &str,
) -> Option<(Arc<LruCacheWithTtl<CacheKey, CacheEntry>>, Duration, bool)> {
    CACHES.get(name).map(|item| {
        (
            item.cache.clone(),
            item.params.ttl,
            item.params.invalidate_with_epoch,
        )
    })
}

declare_metric! {
/// How many times a memoize cache lookup was initiated for a given cache.
///
/// Redundant with the newer [lruttl_lookup_count](lruttl_lookup_count.md) metric.
static CACHE_LOOKUP: CounterVec(
        "memoize_cache_lookup_count",
        &["cache_name"]);
}

declare_metric! {
/// How many times a memoize cache lookup was a hit for a given cache.
///
/// Redundant with the newer [lruttl_hit_count](lruttl_hit_count.md) metric.
static CACHE_HIT: CounterVec(
        "memoize_cache_hit_count",
        &["cache_name"]);
}

declare_metric! {
/// How many times a memoize cache lookup was a miss for a given cache
///
/// Redundant with the newer [lruttl_miss_count](lruttl_miss_count.md) metric.
static CACHE_MISS: CounterVec(
        "memoize_cache_miss_count",
        &["cache_name"]);
}

declare_metric! {
/// How many times a memoize cache lookup resulted in performing the work to populate the entry
///
/// Redundant with the newer [lruttl_populated_count](lruttl_populated_count.md) metric.
static CACHE_POPULATED: CounterVec(
        "memoize_cache_populated_count",
        &["cache_name"]);
}

/// Returns the call arguments as a JSON array with one entry per argument,
/// preserving the argument count, unlike the collapsed JSON value that
/// `multi_value_to_json_value` builds for the cache key.
fn multi_value_to_json_args(lua: &Lua, multi: MultiValue) -> mlua::Result<Vec<serde_json::Value>> {
    multi
        .into_vec()
        .into_iter()
        .map(|v| from_lua_value(lua, v))
        .collect()
}

/// Returns the name of the event handler the current call is running inside,
/// or None when the call is running at top-level policy scope.
fn calling_event_handler(lua: &Lua) -> Option<String> {
    lua.globals().get::<String>("_KUMO_CURRENT_EVENT").ok()
}

fn multi_value_to_json_value(lua: &Lua, multi: MultiValue) -> mlua::Result<serde_json::Value> {
    let mut values = multi.into_vec();
    if values.is_empty() {
        Ok(serde_json::Value::Null)
    } else if values.len() == 1 {
        from_lua_value(lua, values.pop().unwrap())
    } else {
        let mut jvalues = vec![];
        for v in values.into_iter() {
            jvalues.push(from_lua_value(lua, v)?);
        }
        Ok(serde_json::Value::Array(jvalues))
    }
}

/// Looks up `key`, populating it on the calling task on a miss. Has no
/// timeout of its own: a slow populate runs for as long as it takes and its
/// result is still cached. If the caller is cancelled while the populate is
/// running, the populate is aborted and nothing is cached.
async fn populate_inline(
    cache: &LruCacheWithTtl<CacheKey, CacheEntry>,
    key: &CacheKey,
    ttl: Duration,
    lua: &Lua,
    func: Function,
    params: &MultiValue,
    populate_counter: &Counter,
) -> Result<ItemLookup<CacheEntry>, Arc<anyhow::Error>> {
    cache
        .get_or_try_insert(key, |_| ttl, async {
            tracing::trace!("populate {key:?}");
            populate_counter.inc();
            let result: MultiValue = func.call_async(params.clone()).await?;
            CacheEntry::from_multi_value(lua, result)
        })
        .await
}

/// Looks up `key`, populating it on a miss. The populate runs on a task of its
/// own: if the caller's own task is later dropped, such as an HTTP handler
/// whose client disconnected, the populate keeps running to completion and
/// other callers waiting on the same `key` still get its result. `args` must be
/// the JSON form of the call arguments (see `multi_value_to_json_args`), and
/// `registry_name` must name the populate function in the Lua registry. The
/// populate is bounded by `populate_timeout`. Once it elapses, the entry is
/// cached as failed and this call returns that failure as an error, even
/// though the populate task itself keeps running to completion.
async fn populate_detached(
    cache: &LruCacheWithTtl<CacheKey, CacheEntry>,
    key: &CacheKey,
    ttl: Duration,
    cache_name: String,
    registry_name: String,
    args: Vec<serde_json::Value>,
    populate_counter: Counter,
) -> Result<ItemLookup<CacheEntry>, Arc<anyhow::Error>> {
    // populate_timeout reuses the sema timeout of the cache, set from
    // MemoizeParams::populate_timeout. get_or_try_insert_detached hard-cancels
    // the populate once it elapses: it caches a Failed entry with a 60-second
    // TTL in place of a result, and the next lookup spawns a new populate.
    let populate_timeout = cache.get_sema_timeout();
    let make_fut = move || {
        let registry_name = registry_name.clone();
        let args = args.clone();
        let populate_counter = populate_counter.clone();
        let cache_name = cache_name.clone();
        async move {
            let config = load_config().await?;
            let entry = {
                let lua = config.lua()?;
                // The function called here can be the newer policy's version
                // if a reload re-ran the kumo.memoize call before this point.
                // We still cache its result under the key built from
                // epoch_at_start, not the newer epoch: we want a result
                // produced under an older policy never mistaken for one that
                // reflects the current policy, which is what tagging it with
                // the newer epoch would do.
                let func: Function = lua.named_registry_value(&registry_name).map_err(|_| {
                    anyhow::anyhow!(
                        "memoize populate function for cache {cache_name} is not registered in \
                         a freshly loaded config context. This usually means the kumo.memoize \
                         call does not run at top-level policy scope, where a config reload \
                         would re-run it and re-register the function; it can also happen if a \
                         policy reload removed the kumo.memoize call"
                    )
                })?;
                let mut arg_vec = Vec::with_capacity(args.len());
                for a in &args {
                    arg_vec.push(lua.to_value_with(a, serialize_options())?);
                }
                populate_counter.inc();
                let result: MultiValue = func.call_async(MultiValue::from_vec(arg_vec)).await?;
                CacheEntry::from_multi_value(lua, result)?
            };
            config.put();
            Ok::<_, anyhow::Error>(entry)
        }
    };
    cache
        .get_or_try_insert_detached(key, move |_| ttl, make_fut, populate_timeout)
        .await
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "memoize",
        lua.create_function(move |lua, (func, params): (mlua::Function, mlua::Value)| {
            let params: MemoizeParams = from_lua_value(lua, params)?;

            let cache_name = params.name.to_string();

            if !lruttl::is_name_available(&cache_name) {
                return Err(mlua::Error::external(format!(
                    "cannot use name `{cache_name}` for a memoize cache, \
                    as it collides with a built-in cache. \
                    Suggestion: prefix your cache name with `user.` to \
                    avoid conflicts with current and future caches."
                )));
            }

            CACHES.remove_if(&params.name, |_k, item| {
                let changed = item.params != params;
                if changed {
                    tracing::trace!("memoize parameters changed, replacing old cache {params:?}");
                }
                changed
            });
            CACHES.entry(cache_name.to_string()).or_insert_with(|| {
                let cache = LruCacheWithTtl::new(cache_name.clone(), params.capacity);
                if let Some(duration) = params.populate_timeout {
                    cache.set_sema_timeout(duration);
                }
                cache.set_allow_stale_reads(params.allow_stale_reads);

                MemoizeCache {
                    params: params.clone(),
                    cache: Arc::new(cache),
                }
            });

            let lookup_counter = CACHE_LOOKUP
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let hit_counter = CACHE_HIT
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let miss_counter = CACHE_MISS
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let populate_counter = CACHE_POPULATED
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let retry_on_populate_timeout = params.retry_on_populate_timeout;
            let allow_stale_reads = params.allow_stale_reads;
            let detached = match params.detached {
                Some(true) => {
                    if let Some(event) = calling_event_handler(lua) {
                        return Err(mlua::Error::external(format!(
                            "kumo.memoize cache `{cache_name}` sets `detached = true`, but is \
                            being called from within the `{event}` event handler. A detached \
                            populate reloads the policy to re-establish the populate function, \
                            and reloading runs only top-level policy code, not event handlers. \
                            Move this kumo.memoize call to top-level policy scope, or set \
                            `detached = false` if the populate is fast enough to run inline."
                        )));
                    }
                    true
                }
                Some(false) => false,
                None => calling_event_handler(lua).is_none(),
            };

            let registry_name = format!("kumo-memoize-fn.{cache_name}");
            lua.set_named_registry_value(&registry_name, func.clone())?;

            let func_ref = lua.create_registry_value(func)?;

            lua.create_async_function(move |lua, params: MultiValue| {
                let cache_name = cache_name.clone();
                let registry_name = registry_name.clone();
                let func = lua.registry_value::<mlua::Function>(&func_ref);
                let lookup_counter = lookup_counter.clone();
                let hit_counter = hit_counter.clone();
                let miss_counter = miss_counter.clone();
                let populate_counter = populate_counter.clone();
                async move {
                    lookup_counter.inc();
                    let key = multi_value_to_json_value(&lua, params.clone())?;

                    let func = func?;

                    let mut last_failure = None;

                    for _attempt in 0..3 {
                        // We use the epoch from the start of the lookup as part
                        // of the cache key. If the epoch changes while we are in
                        // the middle of computing this value then subsequent calls
                        // through to the cached function will see the newer epoch
                        // and encounter a cache miss. This prevents a race condition
                        // poisoning the cache with a stale value during an epoch
                        // bump. The caller will still observe the stale value, so
                        // ultimately should have some accommodation for detecting
                        // the epoch change and retrying their call through here,
                        // if it is important to not see a stale value.
                        let epoch_at_start = get_current_epoch();

                        let (cache, ttl, invalidate_with_epoch) = get_cache_by_name(&cache_name)
                            .ok_or_else(|| anyhow::anyhow!("cache is somehow undefined!?"))
                            .map_err(any_err)?;

                        let epoch_key = if invalidate_with_epoch && !allow_stale_reads {
                            Some(epoch_at_start)
                        } else {
                            None
                        };
                        let key = serde_json::to_string(&key).map_err(any_err)?;
                        let key = (epoch_key, key);

                        let value_result = if detached {
                            let args = multi_value_to_json_args(&lua, params.clone())?;
                            populate_detached(
                                &cache,
                                &key,
                                ttl,
                                cache_name.clone(),
                                registry_name.clone(),
                                args,
                                populate_counter.clone(),
                            )
                            .await
                        } else {
                            populate_inline(
                                &cache,
                                &key,
                                ttl,
                                &lua,
                                func.clone(),
                                &params,
                                &populate_counter,
                            )
                            .await
                        };

                        match value_result {
                            Ok(lookup) => {
                                if lookup.is_fresh {
                                    miss_counter.inc();
                                } else {
                                    hit_counter.inc();
                                }
                                return lookup.item.to_value(&lua);
                            }
                            Err(err) => {
                                tracing::error!("{cache_name} {key:?} failed: {err:#}");
                                let error = format!("{err:#}");
                                if !retry_on_populate_timeout {
                                    return Err(mlua::Error::external(error));
                                }
                                last_failure.replace(error);
                            }
                        }
                    }

                    Err(mlua::Error::external(
                        last_failure.expect("last_failure to always be set in loop above"),
                    ))
                }
            })
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use mlua::UserDataMethods;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_memoize() {
        let lua = Lua::new();
        register(&lua).unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));

        let globals = lua.globals();
        let counter = Arc::clone(&call_count);
        globals
            .set(
                "do_thing",
                lua.create_function(move |_lua, _: ()| {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    Ok(count)
                })
                .unwrap(),
            )
            .unwrap();

        let result: usize = lua
            .load(
                r#"
            local kumo = require 'kumo';
            -- make cached_do_thing a global for use in the expiry test below
            cached_do_thing = kumo.memoize(do_thing, {
                ttl = "1s",
                capacity = 4,
                name = "test_memoize_do_thing",
                -- bare test Lua has no policy for load_config to reload, so
                -- exercise the inline populate
                detached = false,
            })
            return cached_do_thing() + cached_do_thing() + cached_do_thing()
        "#,
            )
            .eval_async()
            .await
            .unwrap();

        assert_eq!(result, 0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // And confirm that expiry works
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let result: usize = lua
            .load(
                r#"
            return cached_do_thing()
        "#,
            )
            .eval()
            .unwrap();

        assert_eq!(result, 1);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_memoize_rust() {
        let lua = Lua::new();
        register(&lua).unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));

        #[derive(Clone)]
        struct Foo {
            value: usize,
        }

        impl UserData for Foo {
            fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
                Memoized::impl_memoize(methods);
                methods.add_method("get_value", move |_lua, this, _: ()| Ok(this.value));
            }
        }

        let globals = lua.globals();
        let counter = Arc::clone(&call_count);
        globals
            .set(
                "make_foo",
                lua.create_function(move |_lua, _: ()| {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    Ok(Foo { value: count })
                })
                .unwrap(),
            )
            .unwrap();

        let result: usize = lua
            .load(
                r#"
            local kumo = require 'kumo';
            local cached_make_foo = kumo.memoize(make_foo, {
                ttl = "1s",
                capacity = 4,
                name = "test_memoize_make_foo",
                -- bare test Lua has no policy for load_config to reload, so
                -- exercise the inline populate
                detached = false,
            })
            return cached_make_foo():get_value() +
                   cached_make_foo():get_value() +
                   cached_make_foo():get_value()
        "#,
            )
            .eval()
            .unwrap();

        assert_eq!(result, 0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[test_log::test]
    async fn test_memoize_blocked() {
        use std::sync::Mutex;
        use tokio::sync::Notify;

        let call_count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let shared_notify = Arc::new(Mutex::new(Some(Arc::clone(&notify))));

        async fn setup_lua(
            call_count: &Arc<AtomicUsize>,
            notify: Arc<Mutex<Option<Arc<Notify>>>>,
        ) -> Lua {
            let lua = Lua::new();
            register(&lua).unwrap();
            let globals = lua.globals();
            let counter = Arc::clone(&call_count);

            fn take_notify(n: &Arc<Mutex<Option<Arc<Notify>>>>) -> Option<Arc<Notify>> {
                n.lock().unwrap().take()
            }

            globals
                .set(
                    "do_thing",
                    lua.create_async_function(move |_lua, _: ()| {
                        let counter = counter.clone();
                        let notify = notify.clone();
                        async move {
                            eprintln!("do_thing called!");
                            match dbg!(take_notify(&notify)) {
                                Some(notify) => {
                                    eprintln!("do_thing: wait for notify");
                                    notify.notified().await;
                                    eprintln!("notified!");
                                }
                                None => {
                                    eprintln!("do_thing: sleeping");
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    eprintln!("do_thing: slept");
                                }
                            };
                            eprintln!("do_thing: increment");
                            let count = counter.fetch_add(1, Ordering::SeqCst);
                            Ok(count)
                        }
                    })
                    .unwrap(),
                )
                .unwrap();

            let init = r#"
            local kumo = require 'kumo';
            -- make cached_do_thing a global for use in the expiry test below
            cached_do_thing = kumo.memoize(do_thing, {
                ttl = "1s",
                capacity = 4,
                name = "test_memoize_do_thing",
                populate_timeout = "2s",
                -- bare test Lua has no policy for load_config to reload, so
                -- exercise the inline populate
                detached = false,
            })
        "#;

            let () = lua.load(init).eval_async().await.unwrap();
            lua
        }

        let lua = setup_lua(&call_count, shared_notify.clone()).await;
        async fn do_thing(lua: Lua) -> mlua::Result<usize> {
            lua.load("return cached_do_thing()").eval_async().await
        }

        // Set up a future that will get far enough to own the lookup,
        // but that won't complete until we notify it to do so.
        eprintln!("spawning first call to do_thing");
        let first_future = tokio::spawn(do_thing(lua));
        // Let it progress to await on the notifier
        tokio::task::yield_now().await;

        // Now setup a second call; we expect this one to time out
        // because the first one owns the lookup
        let lua = setup_lua(&call_count, shared_notify.clone()).await;
        eprintln!("second call to do_thing");

        let res = do_thing(lua).await;
        eprintln!("second_future is done!");
        let error = res.unwrap_err().to_string();
        assert!(
            error.contains(
                "timed out after 2s on semaphore acquire while waiting for cache to populate"
            ),
            "error: {error}"
        );

        // The third call will succeed because we've de-coupled the
        // original lookup from the herd protection
        eprintln!("third call to do_thing");
        let lua = setup_lua(&call_count, shared_notify.clone()).await;
        let result = do_thing(lua).await.unwrap();
        assert_eq!(result, 0);

        // Now wake up the original future and verify what it returns
        notify.notify_one();
        assert_eq!(first_future.await.unwrap().unwrap(), 1);
    }
}
