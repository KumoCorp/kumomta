use anyhow::Context;
use mlua::{Function, Lua, ToLuaMulti};
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

    lua.globals().set(
        "on",
        lua.create_function(move |lua, (name, func): (String, Function)| {
            let decorated_name = format!("kumomta-on-{}", name);
            lua.set_named_registry_value(&decorated_name, func)?;
            Ok(())
        })?,
    )?;

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
    #[allow(unused)]
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
