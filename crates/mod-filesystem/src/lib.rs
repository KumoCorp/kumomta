use anyhow::anyhow;
use chrono::{DateTime, Utc};
use config::{get_or_create_module, get_or_create_sub_module};
use mlua::{IntoLua, Lua, LuaSerdeExt, Value};
use mod_time::Time;
use serde::Serialize;
use std::time::SystemTime;
use tokio::time::{Duration, Instant};

mod file;

const GLOB_CACHE_CAPACITY: usize = 32;
const DEFAULT_CACHE_TTL: f32 = 60.;

#[derive(Serialize)]
struct FileStat {
    path: String,
    is_file: bool,
    is_dir: bool,
    is_symlink: bool,
    len: u64,
    readonly: bool,
}

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
    fs_mod.set("stat", lua.create_async_function(stat)?)?;
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

async fn stat(lua: Lua, path: String) -> mlua::Result<mlua::Table> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(mlua::Error::external)?;

    let file_type = metadata.file_type();
    let perms = metadata.permissions();

    let stat = FileStat {
        path,
        is_file: file_type.is_file(),
        is_dir: file_type.is_dir(),
        is_symlink: file_type.is_symlink(),
        len: metadata.len(),
        readonly: perms.readonly(),
    };

    match lua.to_value(&stat)? {
        Value::Table(table) => {
            table.set(
                "mtime",
                system_time_to_lua_time(&lua, metadata.modified().ok())?,
            )?;
            table.set(
                "atime",
                system_time_to_lua_time(&lua, metadata.accessed().ok())?,
            )?;
            table.set(
                "ctime",
                system_time_to_lua_time(&lua, metadata.created().ok())?,
            )?;
            Ok(table)
        }
        _ => Err(mlua::Error::external("failed to serialize file metadata")),
    }
}

fn system_time_to_lua_time(lua: &Lua, time: Option<SystemTime>) -> mlua::Result<Value> {
    match time {
        Some(t) => {
            let dt: DateTime<Utc> = t.into();
            Time::from(dt).into_lua(lua)
        }
        None => Ok(Value::Nil),
    }
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

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_basic_operation() -> anyhow::Result<()> {
        let lua = Lua::new();
        register(&lua)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_stat_file() -> anyhow::Result<()> {
        let mut tmp = tempfile::NamedTempFile::new()?;
        tmp.write_all(b"hello world")?;
        let path = tmp.path().to_str().unwrap().to_string();

        let lua = Lua::new();
        let table = stat(lua.clone(), path.clone()).await?;

        let got_path: String = table.get("path")?;
        let is_file: bool = table.get("is_file")?;
        let is_dir: bool = table.get("is_dir")?;
        let is_symlink: bool = table.get("is_symlink")?;
        let len: u64 = table.get("len")?;
        let ctime: mlua::Value = table.get("ctime")?;

        assert_eq!(got_path, path);
        assert!(is_file);
        assert!(!is_dir);
        assert!(!is_symlink);
        assert_eq!(len, 11);
        assert!(!matches!(ctime, mlua::Value::Nil), "ctime is missing");
        Ok(())
    }

    #[tokio::test]
    async fn test_stat_directory() -> anyhow::Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let path = tmp_dir.path().to_str().unwrap().to_string();

        let lua = Lua::new();
        let table = stat(lua.clone(), path.clone()).await?;

        let is_file: bool = table.get("is_file")?;
        let is_dir: bool = table.get("is_dir")?;
        let is_symlink: bool = table.get("is_symlink")?;

        assert!(!is_file);
        assert!(is_dir);
        assert!(!is_symlink);
        Ok(())
    }

    #[tokio::test]
    async fn test_stat_not_found() -> anyhow::Result<()> {
        let lua = Lua::new();
        let result = stat(lua, "/nonexistent/path/that/does/not/exist".to_string()).await;
        assert!(result.is_err());
        Ok(())
    }
}
