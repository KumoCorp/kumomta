use config::{any_err, from_lua_value, get_or_create_module};
use lruttl::LruCacheWithTtl;
use mlua::{Function, Lua, LuaSerdeExt, MultiValue, ToLua, UserData, UserDataMethods};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
#[derive(Clone)]
pub struct Memoized {
    pub to_value: Arc<dyn Fn(&Lua) -> mlua::Result<mlua::Value> + Send + Sync>,
}

impl Memoized {
    /// Call this from your `UserData::add_methods` implementation to
    /// enable memoization for your UserData type
    pub fn impl_memoize<'lua, T, M>(methods: &mut M)
    where
        T: UserData + Send + Sync + Clone + 'static,
        M: UserDataMethods<'lua, T>,
    {
        methods.add_meta_method(
            "__memoize",
            move |_lua, this, _: ()| -> mlua::Result<Memoized> {
                let this = this.clone();
                Ok(Memoized {
                    to_value: Arc::new(move |lua| this.clone().to_lua(lua)),
                })
            },
        );
    }
}

impl UserData for Memoized {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MemoizeParams {
    #[serde(with = "humantime_serde")]
    pub ttl: Duration,
    pub capacity: usize,
    pub name: String,
}

#[derive(Clone)]
enum CacheValue {
    Json(serde_json::Value),
    Memoized(Memoized),
}

impl CacheValue {
    fn from_value(lua: &Lua, value: mlua::Value) -> mlua::Result<Self> {
        match value {
            mlua::Value::UserData(ud) => {
                let mt = ud.get_metatable()?;
                let func: Function = mt.get("__memoize")?;
                let m: Memoized = func.call(mlua::Value::UserData(ud))?;
                Ok(Self::Memoized(m))
            }
            _ => Ok(Self::Json(from_lua_value(lua, value)?)),
        }
    }

    fn to_lua<'lua>(&self, lua: &'lua Lua) -> mlua::Result<mlua::Value<'lua>> {
        match self {
            Self::Json(j) => lua.to_value(j),
            Self::Memoized(m) => (m.to_value)(lua),
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
    fn to_value<'lua>(&self, lua: &'lua Lua) -> mlua::Result<mlua::Value<'lua>> {
        match self {
            Self::Null => Ok(mlua::Value::Nil),
            Self::Single(value) => value.to_lua(lua),
            Self::Multi(values) => {
                let mut result = vec![];
                for v in values {
                    result.push(v.to_lua(lua)?);
                }
                result.to_lua(lua)
            }
        }
    }

    fn from_multi_value(lua: &Lua, multi: MultiValue) -> mlua::Result<Self> {
        let mut values = multi.into_vec();
        if values.is_empty() {
            Ok(Self::Null)
        } else if values.len() == 1 {
            Ok(Self::Single(CacheValue::from_value(
                lua,
                values.pop().unwrap(),
            )?))
        } else {
            let mut cvalues = vec![];
            for v in values.into_iter() {
                cvalues.push(CacheValue::from_value(lua, v)?);
            }
            Ok(Self::Multi(cvalues))
        }
    }
}

struct MemoizeCache {
    params: MemoizeParams,
    cache: Arc<LruCacheWithTtl<String, CacheEntry>>,
}

static CACHES: Lazy<Mutex<HashMap<String, MemoizeCache>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn get_cache_by_name(name: &str) -> Option<(Arc<LruCacheWithTtl<String, CacheEntry>>, Duration)> {
    CACHES
        .lock()
        .unwrap()
        .get(name)
        .map(|item| (item.cache.clone(), item.params.ttl))
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
                        cache: Arc::new(LruCacheWithTtl::new(params.capacity)),
                    },
                );
            }

            let func_ref = lua.create_registry_value(func)?;

            lua.create_async_function(move |lua, params: MultiValue| {
                let cache_name = cache_name.clone();
                let func = lua.registry_value::<mlua::Function>(&func_ref);
                async move {
                    let key = multi_value_to_json_value(lua, params.clone())?;
                    let key = serde_json::to_string(&key).map_err(any_err)?;

                    let (cache, ttl) = get_cache_by_name(&cache_name)
                        .ok_or_else(|| anyhow::anyhow!("cache is somehow undefined!?"))
                        .map_err(any_err)?;

                    if let Some(value) = cache.get(&key) {
                        return Ok(value.to_value(lua)?);
                    }

                    let result: MultiValue = (func?).call_async(params).await?;

                    let value = CacheEntry::from_multi_value(lua, result.clone())?;
                    let return_value = value.to_value(lua)?;

                    cache.insert(key, value, Instant::now() + ttl);

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
            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
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
