use crate::egress_source::{EgressPool, EgressSource};
use crate::queue::QueueConfig;
use crate::smtp_server::{EsmtpDomain, EsmtpListenerParams, RejectError};
use config::{any_err, from_lua_value, get_or_create_module};
use kumo_api_types::egress_path::EgressPathConfig;
use kumo_server_common::http_server::HttpListenerParams;
use kumo_server_lifecycle::ShutdownSubcription;
use kumo_server_runtime::spawn;
use message::Message;
use mlua::prelude::*;
use mlua::{Lua, UserDataMethods, Value};
use throttle::ThrottleSpec;

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

    kumo_mod.set(
        "make_throttle",
        lua.create_function(move |_lua, (name, spec): (String, String)| {
            let spec = ThrottleSpec::try_from(spec.as_str()).map_err(any_err)?;
            let name = format!("lua-user-throttle-{name}");
            Ok(UserThrottle { name, spec })
        })?,
    )?;

    Ok(())
}

#[derive(Clone)]
struct UserThrottle {
    name: String,
    spec: ThrottleSpec,
}

impl LuaUserData for UserThrottle {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("throttle", |lua, this, ()| async move {
            let result = this.spec.throttle(&this.name).await.map_err(any_err)?;
            lua.to_value(&result)
        });

        methods.add_async_method(
            "sleep_if_throttled", |_, this, ()| async move {
                let mut was_throttled = false;

                let mut shutdown = ShutdownSubcription::get();

                loop {
                    let result = this.spec.throttle(&this.name).await.map_err(any_err)?;
                    match result.retry_after {
                        Some(delay) => {
                            was_throttled = true;
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {},
                                _ = shutdown.shutting_down() => {
                                    return Err(mlua::Error::external("sleep_if_throttled: aborted due to shutdown"));
                                }
                            };
                        }
                        None => break
                    }
                }

                Ok(was_throttled)
            }
        );

        methods.add_async_method(
            "delay_message_if_throttled",
            |_, this, msg: Message| async move {
                let result = this.spec.throttle(&this.name).await.map_err(any_err)?;

                match result.retry_after {
                    Some(delay) => {
                        let delay =
                            chrono::Duration::from_std(delay).unwrap_or(kumo_chrono_helper::MINUTE);
                        // We're not using jitter here because the throttle should
                        // ideally result in smooth message flow and the jitter will
                        // (intentionally) perturb that.
                        msg.delay_by(delay).await.map_err(any_err)?;
                        Ok(true)
                    }
                    None => Ok(false),
                }
            },
        );
    }
}
