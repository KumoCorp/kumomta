use crate::egress_source::{EgressPool, EgressSource};
use crate::queue::QueueConfig;
use crate::smtp_server::{EsmtpDomain, EsmtpListenerParams, RejectError};
use config::{any_err, from_lua_value, get_or_create_module};
use kumo_api_types::egress_path::EgressPathConfig;
use kumo_server_common::http_server::HttpListenerParams;
use kumo_server_runtime::spawn;
use mlua::{Lua, Value};

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    crate::queue::GET_Q_CONFIG_SIG.register();
    crate::logging::SHOULD_ENQ_LOG_RECORD_SIG.register();
    crate::PRE_INIT_SIG.register();

    kumo_mod.set(
        "start_http_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: HttpListenerParams = from_lua_value(lua, params)?;
            params
                .start(crate::http_server::make_router())
                .await
                .map_err(any_err)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "start_esmtp_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: EsmtpListenerParams = from_lua_value(lua, params)?;
            spawn("start_esmtp_listener", async move {
                if let Err(err) = params.run().await {
                    tracing::error!("Error in SmtpServer: {err:#}");
                }
            })
            .map_err(any_err)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "reject",
        lua.create_function(move |_lua, (code, message): (u16, String)| {
            Err::<(), mlua::Error>(mlua::Error::external(RejectError { code, message }))
        })?,
    )?;

    kumo_mod.set(
        "make_listener_domain",
        lua.create_function(move |lua, params: Value| {
            let config: EsmtpDomain = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_path",
        lua.create_function(move |lua, params: Value| {
            let config: EgressPathConfig = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_queue_config",
        lua.create_function(move |lua, params: Value| {
            let config: QueueConfig = from_lua_value(lua, params)?;
            Ok(config)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_source",
        lua.create_function(move |lua, params: Value| {
            let source: EgressSource = from_lua_value(lua, params)?;
            Ok(source)
        })?,
    )?;

    kumo_mod.set(
        "make_egress_pool",
        lua.create_function(move |lua, params: Value| {
            let pool: EgressPool = from_lua_value(lua, params)?;
            // pool.register().map_err(any_err)
            Ok(pool)
        })?,
    )?;

    kumo_mod.set(
        "configure_accounting_db_path",
        lua.create_function(|_lua, file_name: String| {
            *crate::accounting::DB_PATH.lock().unwrap() = file_name;
            Ok(())
        })?,
    )?;

    Ok(())
}
