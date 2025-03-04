use config::epoch::{get_current_epoch, ConfigEpoch};
use config::{any_err, from_lua_value, get_or_create_module, serialize_options};
use dashmap::DashMap;
use lruttl::LruCacheWithTtl;
use mlua::{
    FromLua, Function, IntoLua, Lua, LuaSerdeExt, MetaMethod, MultiValue, UserData,
    UserDataMethods, UserDataRef,
};
use prometheus::CounterVec;
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
pub struct MemoizeParams {
    #[serde(with = "duration_serde")]
    pub ttl: Duration,
    pub capacity: usize,
    pub name: String,
    #[serde(default)]
    pub invalidate_with_epoch: bool,
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
                        match state.table().iter().skip(idx).next() {
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

static CACHE_LOOKUP: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_cache_lookup_count",
        "how many times a memoize cache lookup was initiated for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_HIT: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_cache_hit_count",
        "how many times a memoize cache lookup was a hit for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_MISS: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_cache_miss_count",
        "how many times a memoize cache lookup was a miss for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_POPULATED: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_cache_populated_count",
        "how many times a memoize cache lookup resulted in performing the work to populate the entry",
        &["cache_name"]
    )
    .unwrap()
});

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

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "memoize",
        lua.create_function(move |lua, (func, params): (mlua::Function, mlua::Value)| {
            let params: MemoizeParams = from_lua_value(lua, params)?;

            let cache_name = params.name.to_string();

            CACHES.remove_if(&params.name, |_k, item| {
                let changed = item.params != params;
                if changed {
                    tracing::trace!("memoize parameters changed, replacing old cache {params:?}");
                }
                changed
            });
            CACHES
                .entry(cache_name.to_string())
                .or_insert_with(|| MemoizeCache {
                    params: params.clone(),
                    cache: Arc::new(LruCacheWithTtl::new_named(
                        cache_name.clone(),
                        params.capacity,
                    )),
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

            let func_ref = lua.create_registry_value(func)?;

            lua.create_async_function(move |lua, params: MultiValue| {
                let cache_name = cache_name.clone();
                let func = lua.registry_value::<mlua::Function>(&func_ref);
                let lookup_counter = lookup_counter.clone();
                let hit_counter = hit_counter.clone();
                let miss_counter = miss_counter.clone();
                let populate_counter = populate_counter.clone();
                async move {
                    lookup_counter.inc();
                    let key = multi_value_to_json_value(&lua, params.clone())?;
                    let key = serde_json::to_string(&key).map_err(any_err)?;

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

                    let epoch_key = if invalidate_with_epoch {
                        Some(epoch_at_start)
                    } else {
                        None
                    };
                    let key = (epoch_key, key);

                    let value_result = cache
                        .get_or_try_insert(&key, |_| ttl, async {
                            tracing::trace!("populate {key:?}");
                            populate_counter.inc();
                            let result: MultiValue = (func?).call_async(params).await?;
                            CacheEntry::from_multi_value(&lua, result.clone())
                        })
                        .await;

                    match value_result {
                        Ok(lookup) => {
                            if lookup.is_fresh {
                                miss_counter.inc();
                            } else {
                                hit_counter.inc();
                            }
                            lookup.item.to_value(&lua)
                        }
                        Err(err) => {
                            tracing::error!("{cache_name} {key:?} failed: {err:#}");
                            Err(mlua::Error::external(format!("{err:#}")))
                        }
                    }
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
}
