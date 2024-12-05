use config::epoch::{get_current_epoch, ConfigEpoch};
use config::{any_err, from_lua_value, get_or_create_module, serialize_options};
use lruttl::LruCacheWithTtl;
use mlua::{FromLua, Function, IntoLua, Lua, LuaSerdeExt, MultiValue, UserData, UserDataMethods};
use prometheus::CounterVec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

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
}

#[derive(Clone, Hash, Eq, PartialEq)]
pub enum MapKey {
    Integer(mlua::Integer),
    String(Vec<u8>),
}

impl MapKey {
    pub fn as_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        match self {
            Self::Integer(j) => Ok(mlua::Value::Integer(j)),
            Self::String(b) => Ok(mlua::Value::String(lua.create_string(b)?)),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum CacheValue {
    Table(HashMap<MapKey, CacheValue>),
    Json(serde_json::Value),
    Memoized(Memoized),
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
                Ok(Self::Table(map))
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
            Self::Table(m) => {
                let result = lua.create_table()?;
                for (k, v) in m {
                    result.set(k.clone().as_lua(lua)?, v.as_lua(lua)?)?;
                }
                Ok(mlua::Value::Table(result))
            }
        }
    }
}

#[derive(Clone)]
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

static CACHES: LazyLock<Mutex<HashMap<String, MemoizeCache>>> = LazyLock::new(Mutex::default);

type CacheKey = (ConfigEpoch, String);

fn get_cache_by_name(name: &str) -> Option<(Arc<LruCacheWithTtl<CacheKey, CacheEntry>>, Duration)> {
    CACHES
        .lock()
        .unwrap()
        .get(name)
        .map(|item| (item.cache.clone(), item.params.ttl))
}

const REAP_EVERY: usize = 1024;

struct SemaphoreManager {
    map: HashMap<String, Arc<Semaphore>>,
    counter: usize,
}

impl SemaphoreManager {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            counter: 0,
        }
    }

    /// Prune any unreferenced/unusable entries
    fn expire(&mut self) {
        self.map.retain(|_, item| {
            match item.try_acquire() {
                Ok(_) => {
                    // No one is currently using this one, so we can reap it
                    false
                }
                Err(TryAcquireError::Closed) => {
                    // It is not usable, so reap it
                    false
                }
                Err(TryAcquireError::NoPermits) => {
                    // In-use, so we must keep it
                    true
                }
            }
        });
    }

    fn resolve_semaphore(&mut self, name: String) -> Arc<Semaphore> {
        if let Some(s) = self.map.get(&name) {
            if !s.is_closed() {
                return s.clone();
            }
        }

        // To avoid excessive growth (keep in mind that `name` is composed
        // from a cache key which is effectively part of an unbounded,
        // unknowable key space), we occasionally prune out any
        // idle semaphores. We don't do this on every "miss"
        // as the expiration operation is `O(N)` and we'd like
        // to avoid situations where an abusive client can trigger
        // excessive CPU work when running through here.
        // That said, we expect the caller to be responsible for
        // constraining overall concurrency of calls into memoize,
        // as we can't reasonably perform that from this module
        // because we simply do not have enough context to make
        // that work appropriately in every situation.
        // So, this expiration operation is more about keeping the
        // memory overhead reasonably constrained.

        self.counter += 1;
        if self.counter >= REAP_EVERY {
            self.expire();
            self.counter = 0;
        }

        let semaphore = Arc::new(Semaphore::new(1));
        self.map.insert(name, semaphore.clone());
        semaphore
    }
}

static SEMAPHORES: LazyLock<Mutex<SemaphoreManager>> =
    LazyLock::new(|| Mutex::new(SemaphoreManager::new()));

static ACQUIRE_BLOCKED: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_semaphore_acquire_blocked_count",
        "how many times memoize for a specific cache is blocked for concurrent callers",
        &["cache_name"]
    )
    .unwrap()
});
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
static CACHE_MISS_OTHER: LazyLock<CounterVec> = LazyLock::new(|| {
    prometheus::register_counter_vec!(
        "memoize_cache_miss_satisfied_by_other_count",
        "how many times a memoize cache lookup was a miss, but was satisfied while waiting for concurrent callers",
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

/// acquire a semaphore permit for a specific cache and cache key combination.
/// This function will await until the caller is the only caller to hold
/// the semaphore permit.
/// This is used to constrain concurrency of workers on a cache miss
/// and avoid/minimize the thundering herd problem.
async fn acquire_semaphore(
    cache_name: &str,
    key: &CacheKey,
) -> anyhow::Result<OwnedSemaphorePermit> {
    let computed_key = format!("{cache_name}_@_{key:?}");
    let semaphore = SEMAPHORES.lock().unwrap().resolve_semaphore(computed_key);
    match semaphore.clone().try_acquire_owned() {
        Ok(permit) => Ok(permit),
        Err(TryAcquireError::NoPermits) => {
            ACQUIRE_BLOCKED
                .get_metric_with_label_values(&[cache_name])?
                .inc();
            Ok(semaphore.acquire_owned().await?)
        }
        Err(TryAcquireError::Closed) => {
            anyhow::bail!("semaphore for {cache_name} {key:?} is closed!?");
        }
    }
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

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "memoize",
        lua.create_function(move |lua, (func, params): (mlua::Function, mlua::Value)| {
            let params: MemoizeParams = from_lua_value(lua, params)?;

            let cache_name = params.name.to_string();

            let mut caches = CACHES.lock().unwrap();
            let replace = match caches.get_mut(&params.name) {
                Some(existing) => existing.params != params,
                None => true,
            };
            if replace {
                caches.insert(
                    cache_name.to_string(),
                    MemoizeCache {
                        params: params.clone(),
                        cache: Arc::new(LruCacheWithTtl::new_named(
                            cache_name.clone(),
                            params.capacity,
                        )),
                    },
                );
            }

            let lookup_counter = CACHE_LOOKUP
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let hit_counter = CACHE_HIT
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let miss_counter = CACHE_MISS
                .get_metric_with_label_values(&[&cache_name])
                .map_err(any_err)?;
            let miss_other_counter = CACHE_MISS_OTHER
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
                let miss_other_counter = miss_other_counter.clone();
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
                    let key = (epoch_at_start, key);

                    let (cache, ttl) = get_cache_by_name(&cache_name)
                        .ok_or_else(|| anyhow::anyhow!("cache is somehow undefined!?"))
                        .map_err(any_err)?;

                    if let Some(value) = cache.get(&key) {
                        hit_counter.inc();
                        return Ok(value.to_value(&lua)?);
                    }
                    miss_counter.inc();

                    let permit = acquire_semaphore(&cache_name, &key)
                        .await
                        .map_err(any_err)?;

                    // Check cache again in case we raced with someone else
                    // while waiting for the semaphore
                    if let Some(value) = cache.get(&key) {
                        miss_other_counter.inc();
                        return Ok(value.to_value(&lua)?);
                    }

                    populate_counter.inc();

                    let result: MultiValue = (func?).call_async(params).await?;

                    let value = CacheEntry::from_multi_value(&lua, result.clone())?;
                    let return_value = value.to_value(&lua)?;

                    cache.insert(key, value, Instant::now() + ttl);

                    // Explicit release the semaphore to allow others to
                    // also consume the value
                    drop(permit);

                    Ok(return_value)
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

    #[test]
    fn test_memoize() {
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
            .eval()
            .unwrap();

        assert_eq!(result, 0);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // And confirm that expiry works
        std::thread::sleep(std::time::Duration::from_secs(2));

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

    #[test]
    fn test_memoize_rust() {
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
