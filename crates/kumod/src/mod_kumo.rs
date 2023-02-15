use crate::lua_config::get_or_create_module;
use anyhow::Context;
use mlua::{Function, Lua, LuaSerdeExt, Value};
use serde::Deserialize;
use std::path::PathBuf;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "on",
        lua.create_function(move |lua, (name, func): (String, Function)| {
            let decorated_name = format!("kumomta-on-{}", name);
            lua.set_named_registry_value(&decorated_name, func)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "start_esmtp_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params = lua.from_value(params)?;
            tokio::spawn(async move {
                if let Err(err) = start_esmtp_listener(params).await {
                    tracing::error!("Error in SmtpServer: {err:#}");
                }
            });
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "define_spool",
        lua.create_async_function(|lua, params: Value| async move {
            let params = lua.from_value(params)?;
            tokio::spawn(async move {
                if let Err(err) = define_spool(params).await {
                    tracing::error!("Error in spool: {err:#}");
                }
            })
            .await
            .map_err(|err| mlua::Error::external(format!("{err:#}")))
        })?,
    )?;

    Ok(())
}

#[derive(Deserialize)]
struct EsmtpListenerParams {
    #[serde(default = "EsmtpListenerParams::default_listen")]
    listen: String,
    #[serde(default = "EsmtpListenerParams::default_hostname")]
    hostname: String,
}

impl EsmtpListenerParams {
    fn default_listen() -> String {
        "127.0.0.1:2025".to_string()
    }

    fn default_hostname() -> String {
        gethostname::gethostname()
            .to_str()
            .unwrap_or("localhost")
            .to_string()
    }
}

async fn start_esmtp_listener(params: EsmtpListenerParams) -> anyhow::Result<()> {
    use crate::smtp_server::SmtpServer;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&params.listen)
        .await
        .with_context(|| format!("failed to bind to {}", params.listen))?;

    println!("Listening on {}", params.listen);

    loop {
        // The second item contains the IP and port of the new connection.
        let (socket, _) = listener.accept().await?;
        SmtpServer::run(socket, params.hostname.clone()).await?;
    }
}

#[derive(Deserialize)]
struct DefineSpoolParams {
    name: String,
    path: PathBuf,
}

async fn define_spool(params: DefineSpoolParams) -> anyhow::Result<()> {
    crate::spool::SpoolManager::get()
        .await
        .new_local_disk(&params.name, &params.path)
}
