use config::{any_err, from_lua_value, get_or_create_module};
use lruttl::LruCacheWithTtl;
use mlua::{Lua, LuaSerdeExt, MultiValue};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
}

impl CacheValue {
    fn from_value(lua: &Lua, value: mlua::Value) -> mlua::Result<Self> {
        Ok(Self::Json(from_lua_value(lua, value)?))
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
            Self::Single(CacheValue::Json(value)) => lua.to_value(value),
            Self::Multi(values) => {
                let mut result = vec![];
                for v in values {
                    match v {
                        CacheValue::Json(j) => {
                            result.push(j.clone());
                        }
                    }
                }
                let result = serde_json::Value::Array(result);
                lua.to_value(&result)
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
}
