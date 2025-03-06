use crate::egress_source::{EgressPool, EgressSource};
use crate::queue::QueueConfig;
use crate::ready_queue::GET_EGRESS_PATH_CONFIG_SIG;
use crate::smtp_server::{EsmtpDomain, EsmtpListenerParams, RejectError};
use config::{any_err, from_lua_value, get_or_create_module};
use kumo_api_types::egress_path::EgressPathConfig;
use kumo_server_common::http_server::HttpListenerParams;
use kumo_server_lifecycle::ShutdownSubcription;
use message::{EnvelopeAddress, Message};
use mlua::prelude::*;
use mlua::{Lua, UserDataMethods, Value};
use spool::SpoolId;
use std::sync::Arc;
use throttle::ThrottleSpec;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    crate::http_server::admin_suspend_ready_q_v1::register(lua)?;
    crate::http_server::admin_suspend_v1::register(lua)?;
    crate::http_server::admin_bounce_v1::register(lua)?;
    crate::http_server::inject_v1::register(lua)?;

    kumo_mod.set(
        "start_http_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: HttpListenerParams = from_lua_value(&lua, params)?;
            if !config::is_validating() {
                params
                    .start(crate::http_server::make_router(), None)
                    .await
                    .map_err(any_err)?;
            }
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "start_esmtp_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: EsmtpListenerParams = from_lua_value(&lua, params)?;
            if !config::is_validating() {
                params.run().await.map_err(any_err)?;
            }
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_httpinject_threads",
        lua.create_function(move |_, limit: usize| {
            crate::http_server::inject_v1::set_httpinject_threads(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_httpinject_recipient_rate_limit",
        lua.create_function(move |_, spec: Option<String>| {
            let spec = match spec {
                Some(s) => Some(ThrottleSpec::try_from(s).map_err(any_err)?),
                None => None,
            };
            crate::http_server::inject_v1::set_httpinject_recipient_rate_limit(spec);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_smtpsrv_threads",
        lua.create_function(move |_, limit: usize| {
            crate::smtp_server::set_smtpsrv_threads(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_qmaint_threads",
        lua.create_function(move |_, limit: usize| {
            crate::queue::set_qmaint_threads(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_readyq_threads",
        lua.create_function(move |_, limit: usize| {
            crate::ready_queue::set_readyq_threads(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_spoolin_threads",
        lua.create_function(move |_, limit: usize| {
            crate::spool::set_spoolin_threads(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_logging_threads",
        lua.create_function(move |_, limit: usize| {
            crate::logging::set_logging_threads(limit);
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
        "invoke_get_egress_path_config",
        lua.create_async_function(
            |lua, (routing_domain, egress_source, site_name): (String, String, String)| async move {
                let path_config: EgressPathConfig = config::async_call_callback(
                    &lua,
                    &GET_EGRESS_PATH_CONFIG_SIG,
                    (routing_domain, egress_source, site_name),
                )
                .await
                .map_err(any_err)?;
                lua.to_value(&path_config)
            },
        )?,
    )?;

    kumo_mod.set(
        "invoke_get_queue_config",
        lua.create_async_function(|lua, queue_name: String| async move {
            let mut config = config::load_config().await.map_err(any_err)?;
            let queue_config: QueueConfig =
                crate::queue::Queue::call_get_queue_config(&queue_name, &mut config)
                    .await
                    .map_err(any_err)?;
            config.put();
            lua.to_value(&queue_config)
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
            *crate::accounting::DB_PATH.lock() = file_name;
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

    kumo_mod.set(
        "make_message",
        lua.create_function(
            move |_lua, (sender, recip, body): (String, String, mlua::String)| {
                Message::new_dirty(
                    SpoolId::new(),
                    EnvelopeAddress::parse(&sender).map_err(any_err)?,
                    EnvelopeAddress::parse(&recip).map_err(any_err)?,
                    serde_json::json!({}),
                    Arc::new(body.as_bytes().to_vec().into_boxed_slice()),
                )
                .map_err(any_err)
            },
        )?,
    )?;

    Ok(())
}

#[derive(Clone)]
struct UserThrottle {
    name: String,
    spec: ThrottleSpec,
}

impl LuaUserData for UserThrottle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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
