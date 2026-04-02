use anyhow::anyhow;
#[cfg(unix)]
use chrono::{DateTime, Utc};
use config::{get_or_create_module, get_or_create_sub_module};
use mlua::prelude::LuaUserData;
use mlua::{Lua, UserDataFields};
#[cfg(unix)]
use mod_time::Time;
use std::fs::Metadata;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use tokio::time::{Duration, Instant};

mod file;

const GLOB_CACHE_CAPACITY: usize = 32;
const DEFAULT_CACHE_TTL: f32 = 60.;

struct MetadataWrapper(Metadata);

macro_rules! add_metadata_field {
    ($fields:expr, $name:expr, $method:ident) => {
        $fields.add_field_method_get($name, |_, this| Ok(this.0.$method()));
    };
}

impl LuaUserData for MetadataWrapper {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        add_metadata_field!(fields, "is_file", is_file);
        add_metadata_field!(fields, "is_dir", is_dir);
        add_metadata_field!(fields, "is_symlink", is_symlink);
        add_metadata_field!(fields, "len", len); // can be used on non-unix platform
        fields.add_field_method_get("readonly", |_, this| Ok(this.0.permissions().readonly()));

        #[cfg(unix)]
        {
            add_metadata_field!(fields, "dev", dev);
            add_metadata_field!(fields, "ino", ino);
            add_metadata_field!(fields, "mode", mode);
            add_metadata_field!(fields, "nlink", nlink);
            add_metadata_field!(fields, "uid", uid);
            add_metadata_field!(fields, "gid", gid);
            add_metadata_field!(fields, "rdev", rdev);
            add_metadata_field!(fields, "size", size);
            fields.add_field_method_get("atime", |lua, this| {
                let atime_secs = this.0.atime();
                system_time_to_lua_time(lua, Some(atime_secs))
            });
            fields.add_field_method_get("mtime", |lua, this| {
                let mtime_secs = this.0.mtime();
                system_time_to_lua_time(lua, Some(mtime_secs))
            });
            fields.add_field_method_get("ctime", |lua, this| {
                let ctime_secs = this.0.ctime();
                system_time_to_lua_time(lua, Some(ctime_secs))
            });
            add_metadata_field!(fields, "blksize", blksize);
            add_metadata_field!(fields, "blocks", blocks);
        }
    }
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
    fs_mod.set(
        "metadata_for_path",
        lua.create_async_function(metadata_for_path)?,
    )?;
    fs_mod.set(
        "symlink_metadata_for_path",
        lua.create_async_function(symlink_metadata_for_path)?,
    )?;
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

async fn metadata_for_path(_lua: Lua, path: String) -> mlua::Result<MetadataWrapper> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(mlua::Error::external)?;
    Ok(MetadataWrapper(metadata))
}

async fn symlink_metadata_for_path(_lua: Lua, path: String) -> mlua::Result<MetadataWrapper> {
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(mlua::Error::external)?;
    Ok(MetadataWrapper(metadata))
}

#[cfg(unix)]
fn system_time_to_lua_time(_lua: &Lua, epoch_secs: Option<i64>) -> mlua::Result<Option<Time>> {
    Ok(epoch_secs.map(|secs| {
        let dt = DateTime::<Utc>::from_timestamp(secs, 0).unwrap();
        Time::from(dt)
    }))
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
    async fn test_metadata_file() -> anyhow::Result<()> {
        let mut tmp = tempfile::NamedTempFile::new()?;
        tmp.write_all(b"hello world")?;
        let path = tmp.path().to_str().unwrap().to_string();

        let lua = Lua::new();
        register(&lua)?;

        lua.globals().set("path", path)?;
        lua.load(
            r#"
local kumo = require 'kumo'
local meta = kumo.fs.metadata_for_path(path)
assert(meta.is_file)
assert(not meta.is_dir)
assert(not meta.is_symlink)
assert(meta.len == 11)

if meta.size ~= nil then
    assert(meta.size == 11)
end

if meta.ino ~= nil then
    assert(meta.ino > 0)
end

if meta.dev ~= nil then
    assert(meta.dev > 0)
end

if meta.mode ~= nil then
    assert(meta.mode > 0)
end
"#,
        )
        .exec_async()
        .await?;

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_metadata_symlink() -> anyhow::Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let target = tmp_dir.path().join("target.txt");
        let link = tmp_dir.path().join("link.txt");

        std::fs::write(&target, b"hello world")?;
        std::os::unix::fs::symlink(&target, &link)?;

        let lua = Lua::new();
        register(&lua)?;

        let link_path = link.to_str().unwrap().to_string();
        lua.globals().set("path", link_path)?;
        lua.load(
            r#"
local kumo = require 'kumo'
local meta = kumo.fs.metadata_for_path(path)
assert(meta.is_file)
assert(not meta.is_dir)
assert(not meta.is_symlink) -- this is false cause metadata_for_path follows symlinks
assert(meta.size == 11)
assert(meta.len == 11)

local symlink_meta = kumo.fs.symlink_metadata_for_path(path)
assert(not symlink_meta.is_file)
assert(not symlink_meta.is_dir)
assert(symlink_meta.is_symlink)
"#,
        )
        .exec_async()
        .await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_metadata_directory() -> anyhow::Result<()> {
        let tmp_dir = tempfile::tempdir()?;
        let path = tmp_dir.path().to_str().unwrap().to_string();

        let lua = Lua::new();
        register(&lua)?;

        lua.globals().set("path", path)?;
        lua.load(
            r#"
local kumo = require 'kumo'
local meta = kumo.fs.metadata_for_path(path)
assert(not meta.is_file)
assert(meta.is_dir)
assert(not meta.is_symlink)
assert(meta.len > 0)

if meta.size ~= nil then
    assert(meta.size > 0)
end

if meta.ino ~= nil then
    assert(meta.ino > 0)
end

if meta.dev ~= nil then
    assert(meta.dev > 0)
end

if meta.mode ~= nil then
    assert(meta.mode > 0)
end
"#,
        )
        .exec_async()
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_metadata_not_found() -> anyhow::Result<()> {
        let lua = Lua::new();
        register(&lua)?;
        lua.globals().set("path", "this/path/does/not/exist")?;
        lua.load(
            r#"
local kumo = require 'kumo'
local ok, meta = pcall(kumo.fs.metadata_for_path, path)
assert(not ok)
assert(string.match(tostring(meta), "No such file or directory"))
"#,
        )
        .exec_async()
        .await?;
        Ok(())
    }
}
