use config::{any_err, get_or_create_module};
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

struct MemoizeCache {
    params: MemoizeParams,
    cache: Arc<LruCacheWithTtl<String, serde_json::Value>>,
}

static CACHES: Lazy<Mutex<HashMap<String, MemoizeCache>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn get_cache_by_name(
    name: &str,
) -> Option<(Arc<LruCacheWithTtl<String, serde_json::Value>>, Duration)> {
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
        lua.from_value(values.pop().unwrap())
    } else {
        let mut jvalues = vec![];
        for v in values.into_iter() {
            jvalues.push(lua.from_value(v)?);
        }
        Ok(serde_json::Value::Array(jvalues))
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "memoize",
        lua.create_function(move |lua, (func, params): (mlua::Function, mlua::Value)| {
            let params: MemoizeParams = lua.from_value(params)?;

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
                        return Ok(lua.to_value(&value)?);
                    }

                    let result: MultiValue = (func?).call_async(params).await?;

                    let value = multi_value_to_json_value(lua, result.clone())?;
                    let return_value = lua.to_value(&value)?;

                    cache.insert(key, value, Instant::now() + ttl);

                    Ok(return_value)
                }
            })
        })?,
    )?;

    Ok(())
}
