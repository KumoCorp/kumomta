use anyhow::anyhow;
use config::{get_or_create_module, get_or_create_sub_module};
use mlua::Lua;
use tokio::time::{Duration, Instant};

mod file;

const GLOB_CACHE_CAPACITY: usize = 32;
const DEFAULT_CACHE_TTL: f32 = 60.;

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
struct GlobKey {
    pattern: String,
    path: Option<String>,
}

lruttl::declare_cache! {
/// Caches glob results by glob pattern
static CACHE: LruCacheWithTtl<GlobKey, Result<Vec<String>, String>>::new("mod_filesystem_glob_cache", GLOB_CACHE_CAPACITY);
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    kumo_mod.set("read_dir", lua.create_async_function(read_dir)?)?;
    kumo_mod.set("glob", lua.create_async_function(cached_glob)?)?;
    kumo_mod.set("uncached_glob", lua.create_async_function(uncached_glob)?)?;

    let fs_mod = get_or_create_sub_module(lua, "fs")?;
    fs_mod.set("open", lua.create_async_function(file::AsyncFile::open)?)?;
    fs_mod.set("read_dir", lua.create_async_function(read_dir)?)?;
    fs_mod.set("glob", lua.create_async_function(cached_glob)?)?;
    fs_mod.set("uncached_glob", lua.create_async_function(uncached_glob)?)?;

    Ok(())
}

async fn read_dir(_: Lua, path: String) -> mlua::Result<Vec<String>> {
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

async fn cached_glob(
    _: Lua,
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
        .await
        .map_err(mlua::Error::external)
}

async fn uncached_glob(
    _: Lua,
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
