use anyhow::anyhow;
use config::get_or_create_module;
use lruttl::LruCacheWithTtl;
use mlua::Lua;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const GLOB_CACHE_CAPACITY: usize = 32;
const DEFAULT_CACHE_TTL: f32 = 60.;

#[derive(PartialEq, Eq, Hash)]
struct GlobKey {
    pattern: String,
    path: Option<String>,
}

static CACHE: LazyLock<Arc<LruCacheWithTtl<GlobKey, Result<Vec<String>, String>>>> =
    LazyLock::new(|| make_cache());

fn make_cache() -> Arc<LruCacheWithTtl<GlobKey, Result<Vec<String>, String>>> {
    Arc::new(LruCacheWithTtl::new_named(
        "mod_filesystem_glob_cache",
        GLOB_CACHE_CAPACITY,
    ))
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    kumo_mod.set("read_dir", lua.create_async_function(read_dir)?)?;
    kumo_mod.set("glob", lua.create_async_function(cached_glob)?)?;
    kumo_mod.set("uncached_glob", lua.create_async_function(uncached_glob)?)?;
    Ok(())
}

async fn read_dir<'lua>(_: &'lua Lua, path: String) -> mlua::Result<Vec<String>> {
    let mut dir = tokio::fs::read_dir(path)
        .await
        .map_err(mlua::Error::external)?;
    let mut entries = vec![];
    while let Some(entry) = dir.next_entry().await.map_err(mlua::Error::external)? {
        if let Some(utf8) = entry.path().to_str() {
            entries.push(utf8.to_string());
        } else {
            return Err(mlua::Error::external(anyhow!(
                "path entry {} is not representable as utf8",
                entry.path().display()
            )));
        }
    }
    Ok(entries)
}

async fn cached_glob<'lua>(
    _: &'lua Lua,
    (pattern, path, ttl): (String, Option<String>, Option<f32>),
) -> mlua::Result<Vec<String>> {
    let key = GlobKey {
        pattern: pattern.to_string(),
        path: path.clone(),
    };
    if let Some(cached) = CACHE.get(&key) {
        return cached.map_err(mlua::Error::external);
    }

    let result = glob(pattern.clone(), path.clone())
        .await
        .map_err(|err| format!("glob({pattern}, {path:?}): {err:#}"));

    let ttl = Duration::from_secs_f32(ttl.unwrap_or(DEFAULT_CACHE_TTL));

    CACHE
        .insert(key, result.clone(), Instant::now() + ttl)
        .map_err(mlua::Error::external)
}

async fn uncached_glob<'lua>(
    _: &'lua Lua,
    (pattern, path): (String, Option<String>),
) -> mlua::Result<Vec<String>> {
    glob(pattern, path).await
}

async fn glob(pattern: String, path: Option<String>) -> mlua::Result<Vec<String>> {
    let entries = tokio::task::spawn_blocking(move || {
        let mut entries = vec![];
        let glob = filenamegen::Glob::new(&pattern)?;
        for path in glob.walk(path.as_deref().unwrap_or(".")) {
            if let Some(utf8) = path.to_str() {
                entries.push(utf8.to_string());
            } else {
                return Err(anyhow!(
                    "path entry {} is not representable as utf8",
                    path.display()
                ));
            }
        }
        Ok(entries)
    })
    .await
    .map_err(mlua::Error::external)?
    .map_err(mlua::Error::external)?;
    Ok(entries)
}
