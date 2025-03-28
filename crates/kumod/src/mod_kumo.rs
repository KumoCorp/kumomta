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
use num_format::{Locale, ToFormattedString};
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
        "set_timeq_spawn_reinsertion",
        lua.create_function(move |_, v: bool| {
            crate::queue::maintainer::set_spawn_reinsertion(v);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_qmaint_threads",
        lua.create_function(move |_, limit: usize| {
            crate::queue::maintainer::set_qmaint_threads(limit);
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

fn explain_throttle(spec: &ThrottleSpec) -> String {
    let burst = spec.burst();
    let interval = spec.interval();

    let spec_rate = spec.limit as f64 / spec.period as f64;
    let burst_rate = burst as f64 / interval.as_secs_f64();

    fn number(n: f64) -> String {
        if n < 1.0 {
            format!("{n:.3}")
        } else {
            format!("{}", (n.ceil() as usize).to_formatted_string(&Locale::en))
        }
    }

    fn rates(rate: f64) -> String {
        let per_second = rate;
        let per_minute = per_second * 60.0;
        let per_hour = per_minute * 60.0;
        let per_day = per_hour * 24.0;

        let per_second = number(per_second);
        let per_minute = number(per_minute);
        let per_hour = number(per_hour);
        let per_day = number(per_day);

        format!("{per_second}/s, {per_minute}/m, {per_hour}/h, {per_day}/d")
    }

    let mut result = spec.to_string();

    if spec.max_burst.is_none() {
        result.push_str("\nimplied burst rate ");
    } else {
        result.push_str("\nexplicit burst rate ");
    }
    result.push_str(&format!(
        "{burst} every {}, or: {} in that period.\n",
        humantime::format_duration(interval),
        rates(burst_rate)
    ));

    result.push_str(&format!("overall rate {}", rates(spec_rate)));
    result
}

#[cfg(test)]
#[test]
fn test_explain_throttle() {
    k9::snapshot!(
        explain_throttle(&ThrottleSpec::try_from("100/h").unwrap()),
        "
100/h
implied burst rate 100 every 36s, or: 3/s, 167/m, 10,000/h, 240,000/d in that period.
overall rate 0.028/s, 2/m, 100/h, 2,400/d
"
    );
    k9::snapshot!(
        explain_throttle(&ThrottleSpec::try_from("500/d").unwrap()),
        "
500/d
implied burst rate 500 every 2m 52s 800ms, or: 3/s, 174/m, 10,417/h, 250,000/d in that period.
overall rate 0.006/s, 0.347/m, 21/h, 500/d
"
    );
    k9::snapshot!(
        explain_throttle(&ThrottleSpec::try_from("500/d,max_burst=1").unwrap()),
        "
500/d,max_burst=1
explicit burst rate 1 every 2m 52s 800ms, or: 0.006/s, 0.347/m, 21/h, 500/d in that period.
overall rate 0.006/s, 0.347/m, 21/h, 500/d
"
    );
    k9::snapshot!(
        explain_throttle(&ThrottleSpec::try_from("500/d,max_burst=10").unwrap()),
        "
500/d,max_burst=10
explicit burst rate 10 every 2m 52s 800ms, or: 0.058/s, 4/m, 209/h, 5,000/d in that period.
overall rate 0.006/s, 0.347/m, 21/h, 500/d
"
    );
}

impl LuaUserData for UserThrottle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("throttle", |lua, this, ()| async move {
            let result = this.spec.throttle(&this.name).await.map_err(any_err)?;
            lua.to_value(&result)
        });

        methods.add_method("explain", move |_, this, ()| {
            Ok(explain_throttle(&this.spec))
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
