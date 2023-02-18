use crate::dest_site::DestSiteConfig;
use crate::lua_config::get_or_create_module;
use crate::queue::QueueConfig;
use crate::smtp_server::{EsmtpListenerParams, RejectError};
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

    kumo_mod.set(
        "reject",
        lua.create_function(move |_lua, (code, message): (u16, String)| {
            Err::<(), mlua::Error>(mlua::Error::external(RejectError { code, message }))
        })?,
    )?;

    kumo_mod.set(
        "make_site_config",
        lua.create_function(move |lua, params: Value| {
            let config: DestSiteConfig = lua.from_value(params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_queue_config",
        lua.create_function(move |lua, params: Value| {
            let config: QueueConfig = lua.from_value(params)?;
            Ok(config)
        })?,
    )?;

    Ok(())
}

async fn start_esmtp_listener(params: EsmtpListenerParams) -> anyhow::Result<()> {
    use crate::smtp_server::SmtpServer;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(&params.listen)
        .await
        .with_context(|| format!("failed to bind to {}", params.listen))?;

    println!("Listening on {}", params.listen);

    loop {
        let (socket, peer_address) = listener.accept().await?;
        let params = params.clone();
        crate::runtime::Runtime::run(move || {
            tokio::task::spawn_local(async move {
                if let Err(err) = SmtpServer::run(socket, peer_address, params).await {
                    tracing::error!("SmtpServer::run: {err:#}");
                }
            });
        })
        .await?;
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
