use anyhow::Context;
use mlua::{Lua, Table, ToLuaMulti, Value};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug)]
pub struct LuaConfig {
    lua: Lua,
}

lazy_static::lazy_static! {
    static ref POLICY_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
}

pub async fn set_policy_path(path: PathBuf) -> anyhow::Result<()> {
    POLICY_FILE.lock().unwrap().replace(path);
    load_config().await?;
    Ok(())
}

fn get_policy_path() -> Option<PathBuf> {
    POLICY_FILE.lock().unwrap().clone()
}

pub async fn load_config() -> anyhow::Result<LuaConfig> {
    let lua = Lua::new();

    crate::mod_kumo::register(&lua)?;

    if let Some(policy) = get_policy_path() {
        let code = tokio::fs::read_to_string(&policy)
            .await
            .with_context(|| format!("reading policy file {policy:?}"))?;

        let func = {
            let chunk = lua.load(&code);
            let chunk = chunk.set_name(policy.to_string_lossy())?;
            chunk.into_function()?
        };

        func.call(())?;
    }

    Ok(LuaConfig { lua })
}

impl LuaConfig {
    /// Call a callback registered via `on`.
    ///
    /// I'd love to use this, but unfortunately, this is a !Send future
    /// due to limitations of mlua and it can't be used within a tokio::spawn'd
    /// block.
    /// An alternative is to use `smol` instead of `tokio`, and do some more
    /// plumbing to set up a thread pool + queue for handling incoming connections,
    /// but for now we just use the synchronous call_callback method below:
    /// it shouldn't matter much for sender-focused deployments
    pub async fn async_call_callback<'lua, S: AsRef<str>, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<()> {
        let name = name.as_ref();
        let decorated_name = format!("kumomta-on-{}", name);
        match self
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => {
                func.call_async(args.clone()).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Call a callback registered via `on`.
    pub fn call_callback<'lua, S: AsRef<str>, A: ToLuaMulti<'lua> + Clone>(
        &'lua mut self,
        name: S,
        args: A,
    ) -> anyhow::Result<()> {
        let name = name.as_ref();
        let decorated_name = format!("kumomta-on-{}", name);
        match self
            .lua
            .named_registry_value::<_, mlua::Function>(&decorated_name)
        {
            Ok(func) => {
                func.call(args.clone())?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

pub fn get_or_create_module<'lua>(lua: &'lua Lua, name: &str) -> anyhow::Result<mlua::Table<'lua>> {
    let globals = lua.globals();
    let package: Table = globals.get("package")?;
    let loaded: Table = package.get("loaded")?;

    let module = loaded.get(name)?;
    match module {
        Value::Nil => {
            let module = lua.create_table()?;
            loaded.set(name, module.clone())?;
            Ok(module)
        }
        Value::Table(table) => Ok(table),
        wat => anyhow::bail!(
            "cannot register module {} as package.loaded.{} is already set to a value of type {}",
            name,
            name,
            wat.type_name()
        ),
    }
}

pub fn get_or_create_sub_module<'lua>(
    lua: &'lua Lua,
    name: &str,
) -> anyhow::Result<mlua::Table<'lua>> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    let sub = kumo_mod.get(name)?;
    match sub {
        Value::Nil => {
            let sub = lua.create_table()?;
            kumo_mod.set(name, sub.clone())?;
            Ok(sub)
        }
        Value::Table(sub) => Ok(sub),
        wat => anyhow::bail!(
            "cannot register module kumo.{name} as it is already set to a value of type {}",
            wat.type_name()
        ),
    }
}
