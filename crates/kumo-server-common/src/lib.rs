use config::{
    any_err, decorate_callback_name, from_lua_value, get_or_create_module, load_config,
    serialize_options, CallbackSignature,
};
use kumo_server_runtime::available_parallelism;
use mlua::{Function, Lua, LuaSerdeExt, Value, Variadic};
use mod_redis::RedisConnKey;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicUsize;

pub mod acct;
pub mod authn_authz;
pub mod config_handle;
pub mod diagnostic_logging;
pub mod disk_space;
pub mod http_server;
pub mod log;
pub mod nodeid;
pub mod panic;
pub mod start;
pub mod tls_helpers;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    for func in [
        mod_redis::register,
        data_loader::register,
        mod_digest::register,
        mod_encode::register,
        mod_aws_sigv4::register,
        cidr_map::register,
        domain_map::register,
        mod_amqp::register,
        mod_filesystem::register,
        mod_file_type::register,
        mod_http::register,
        mod_regex::register,
        mod_serde::register,
        mod_sqlite::register,
        mod_crypto::register,
        mod_smtp_response_normalize::register,
        mod_string::register,
        mod_time::register,
        mod_dns_resolver::register,
        mod_kafka::register,
        mod_memoize::register,
        mod_mimepart::register,
        mod_mpsc::register,
        mod_nats::register,
        mod_uuid::register,
        kumo_api_types::shaping::register,
        regex_set_map::register,
        crate::authn_authz::register,
    ] {
        func(lua)?;
    }

    let kumo_mod = get_or_create_module(lua, "kumo")?;
    kumo_mod.set("version", version_info::kumo_version())?;

    fn event_registrar_name(name: &str) -> String {
        format!("kumomta-event-registrars-{name}")
    }

    // Record the call stack of the code calling kumo.on so that
    // kumo.get_event_registrars can retrieve it later
    fn register_event_caller(lua: &Lua, name: &str) -> mlua::Result<()> {
        let decorated_name = event_registrar_name(name);
        let mut call_stack = vec![];
        for n in 1.. {
            match lua.inspect_stack(n, |info| {
                let source = info.source();
                format!(
                    "{}:{}",
                    source
                        .short_src
                        .as_ref()
                        .map(|b| b.to_string())
                        .unwrap_or_else(String::new),
                    info.current_line().unwrap_or(0)
                )
            }) {
                Some(info) => {
                    call_stack.push(info);
                }
                None => break,
            }
        }

        let tbl: Value = lua.named_registry_value(&decorated_name)?;
        match tbl {
            Value::Nil => {
                let tbl = lua.create_table()?;
                tbl.set(1, call_stack)?;
                lua.set_named_registry_value(&decorated_name, tbl)?;
                Ok(())
            }
            Value::Table(tbl) => {
                let len = tbl.raw_len();
                tbl.set(len + 1, call_stack)?;
                Ok(())
            }
            _ => Err(mlua::Error::external(format!(
                "registry key for {decorated_name} has invalid type",
            ))),
        }
    }

    // Returns the list of call-stacks of the code that registered
    // for a specific named event
    kumo_mod.set(
        "get_event_registrars",
        lua.create_function(move |lua, name: String| {
            let decorated_name = event_registrar_name(&name);
            let value: Value = lua.named_registry_value(&decorated_name)?;
            Ok(value)
        })?,
    )?;

    kumo_mod.set(
        "on",
        lua.create_function(move |lua, (name, func): (String, Function)| {
            let decorated_name = decorate_callback_name(&name);

            if let Ok(current_event) = lua.globals().get::<String>("_KUMO_CURRENT_EVENT") {
                if current_event != "main" {
                    return Err(mlua::Error::external(format!(
                        "Attempting to register an event handler via \
                    `kumo.on('{name}', ...)` from within the event handler \
                    '{current_event}'. You must move your event handler registration \
                    so that it is setup directly when the policy is loaded \
                    in order for it to consistently trigger and handle events."
                    )));
                }
            }

            register_event_caller(lua, &name)?;

            if config::does_callback_allow_multiple(&name) {
                let tbl: Value = lua.named_registry_value(&decorated_name)?;
                return match tbl {
                    Value::Nil => {
                        let tbl = lua.create_table()?;
                        tbl.set(1, func)?;
                        lua.set_named_registry_value(&decorated_name, tbl)?;
                        Ok(())
                    }
                    Value::Table(tbl) => {
                        let len = tbl.raw_len();
                        tbl.set(len + 1, func)?;
                        Ok(())
                    }
                    _ => Err(mlua::Error::external(format!(
                        "registry key for {decorated_name} has invalid type",
                    ))),
                };
            }

            let existing: Value = lua.named_registry_value(&decorated_name)?;
            match existing {
                Value::Nil => {}
                Value::Function(func) => {
                    let info = func.info();
                    let src = info.source.unwrap_or_else(|| "?".into());
                    let line = info.line_defined.unwrap_or(0);
                    return Err(mlua::Error::external(format!(
                        "{name} event already has a handler defined at {src}:{line}"
                    )));
                }
                _ => {
                    return Err(mlua::Error::external(format!(
                        "{name} event already has a handler"
                    )));
                }
            }

            lua.set_named_registry_value(&decorated_name, func)?;
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_diagnostic_log_filter",
        lua.create_function(move |_, filter: String| {
            diagnostic_logging::set_diagnostic_log_filter(&filter).map_err(any_err)
        })?,
    )?;

    fn variadic_to_string(args: Variadic<Value>) -> String {
        let mut output = String::new();
        for (idx, item) in args.into_iter().enumerate() {
            if idx > 0 {
                output.push(' ');
            }

            match item {
                Value::String(s) => match s.to_str() {
                    Ok(s) => output.push_str(&s),
                    Err(_) => {
                        let item = s.to_string_lossy();
                        output.push_str(&item);
                    }
                },
                item => match item.to_string() {
                    Ok(s) => output.push_str(&s),
                    Err(_) => output.push_str(&format!("{item:?}")),
                },
            }
        }
        output
    }

    fn get_caller(lua: &Lua) -> String {
        match lua.inspect_stack(1, |info| {
            let source = info.source();
            let file_name = source
                .short_src
                .as_ref()
                .map(|b| b.to_string())
                .unwrap_or_else(String::new);
            // Lua returns the somewhat obnoxious `[string "source.lua"]`
            // Let's fix that up to be a bit nicer
            let file_name = match file_name.strip_prefix("[string \"") {
                Some(name) => name.strip_suffix("\"]").unwrap_or(name),
                None => &file_name,
            };

            format!("{file_name}:{}", info.current_line().unwrap_or(0))
        }) {
            Some(info) => info,
            None => "?".to_string(),
        }
    }

    kumo_mod.set(
        "log_error",
        lua.create_function(move |lua, args: Variadic<Value>| {
            if tracing::event_enabled!(target: "lua", tracing::Level::ERROR) {
                let src = get_caller(lua);
                tracing::error!(target: "lua", "{src}: {}", variadic_to_string(args));
            }
            Ok(())
        })?,
    )?;
    kumo_mod.set(
        "log_info",
        lua.create_function(move |lua, args: Variadic<Value>| {
            if tracing::event_enabled!(target: "lua", tracing::Level::INFO) {
                let src = get_caller(lua);
                tracing::info!(target: "lua", "{src}: {}", variadic_to_string(args));
            }
            Ok(())
        })?,
    )?;
    kumo_mod.set(
        "log_warn",
        lua.create_function(move |lua, args: Variadic<Value>| {
            if tracing::event_enabled!(target: "lua", tracing::Level::WARN) {
                let src = get_caller(lua);
                tracing::warn!(target: "lua", "{src}: {}", variadic_to_string(args));
            }
            Ok(())
        })?,
    )?;
    kumo_mod.set(
        "log_debug",
        lua.create_function(move |lua, args: Variadic<Value>| {
            if tracing::event_enabled!(target: "lua", tracing::Level::DEBUG) {
                let src = get_caller(lua);
                tracing::debug!(target: "lua", "{src}: {}", variadic_to_string(args));
            }
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_max_spare_lua_contexts",
        lua.create_function(move |_, limit: usize| {
            config::set_max_spare(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_max_lua_context_use_count",
        lua.create_function(move |_, limit: usize| {
            config::set_max_use(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_max_lua_context_age",
        lua.create_function(move |_, limit: usize| {
            config::set_max_age(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_lua_gc_on_put",
        lua.create_function(move |_, enable: u8| {
            config::set_gc_on_put(enable);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_lruttl_cache_capacity",
        lua.create_function(move |_, (name, capacity): (String, usize)| {
            if lruttl::set_cache_capacity(&name, capacity) {
                Ok(())
            } else {
                Err(mlua::Error::external(format!(
                    "could not set capacity for cache {name} \
                    as that is not a pre-defined lruttl cache name"
                )))
            }
        })?,
    )?;

    kumo_mod.set(
        "set_config_monitor_globs",
        lua.create_function(move |_, globs: Vec<String>| {
            config::epoch::set_globs(globs).map_err(any_err)?;
            Ok(())
        })?,
    )?;
    kumo_mod.set(
        "eval_config_monitor_globs",
        lua.create_async_function(|_, _: ()| async move {
            config::epoch::eval_globs().await.map_err(any_err)
        })?,
    )?;
    kumo_mod.set(
        "bump_config_epoch",
        lua.create_function(move |_, _: ()| {
            config::epoch::bump_current_epoch();
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "available_parallelism",
        lua.create_function(move |_, _: ()| available_parallelism().map_err(any_err))?,
    )?;

    kumo_mod.set(
        "set_memory_hard_limit",
        lua.create_function(move |_, limit: usize| {
            kumo_server_memory::set_hard_limit(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_memory_low_thresh",
        lua.create_function(move |_, limit: usize| {
            kumo_server_memory::set_low_memory_thresh(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "set_memory_soft_limit",
        lua.create_function(move |_, limit: usize| {
            kumo_server_memory::set_soft_limit(limit);
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "configure_redis_throttles",
        lua.create_async_function(|lua, params: Value| async move {
            let key: RedisConnKey = from_lua_value(&lua, params)?;
            let conn = key.open().map_err(any_err)?;
            conn.ping().await.map_err(any_err)?;
            throttle::use_redis(conn).await.map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "traceback",
        lua.create_function(move |lua: &Lua, level: usize| {
            #[derive(Debug, Serialize)]
            struct Frame {
                event: String,
                name: Option<String>,
                name_what: Option<String>,
                source: Option<String>,
                short_src: Option<String>,
                line_defined: Option<usize>,
                last_line_defined: Option<usize>,
                what: &'static str,
                curr_line: Option<usize>,
                is_tail_call: bool,
            }

            let mut frames = vec![];
            for n in level.. {
                match lua.inspect_stack(n, |info| {
                    let source = info.source();
                    let names = info.names();
                    Frame {
                        curr_line: info.current_line(),
                        is_tail_call: info.is_tail_call(),
                        event: format!("{:?}", info.event()),
                        last_line_defined: source.last_line_defined,
                        line_defined: source.line_defined,
                        name: names.name.as_ref().map(|b| b.to_string()),
                        name_what: names.name_what.as_ref().map(|b| b.to_string()),
                        source: source.source.as_ref().map(|b| b.to_string()),
                        short_src: source.short_src.as_ref().map(|b| b.to_string()),
                        what: source.what,
                    }
                }) {
                    Some(frame) => {
                        frames.push(frame);
                    }
                    None => break,
                }
            }

            lua.to_value(&frames)
        })?,
    )?;

    // TODO: options like restarting on error, delay between
    // restarts and so on
    #[derive(Deserialize, Debug)]
    struct TaskParams {
        event_name: String,
        args: Vec<serde_json::Value>,
    }

    impl TaskParams {
        async fn run(&self) -> anyhow::Result<()> {
            let mut config = load_config().await?;

            let sig = CallbackSignature::<Value, ()>::new(self.event_name.to_string());

            config
                .convert_args_and_call_callback(&sig, &self.args)
                .await?;

            config.put();

            Ok(())
        }
    }

    kumo_mod.set(
        "spawn_task",
        lua.create_function(|lua, params: Value| {
            let params: TaskParams = lua.from_value(params)?;

            if !config::is_validating() {
                std::thread::Builder::new()
                    .name(format!("spawned-task-{}", params.event_name))
                    .spawn(move || {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_io()
                            .enable_time()
                            .on_thread_park(kumo_server_memory::purge_thread_cache)
                            .build()
                            .unwrap();
                        let event_name = params.event_name.clone();

                        let result = runtime.block_on(async move { params.run().await });
                        if let Err(err) = result {
                            tracing::error!("Error while dispatching {event_name}: {err:#}");
                        }
                    })?;
            }

            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "spawn_thread_pool",
        lua.create_function(|lua, params: Value| {
            #[derive(Deserialize, Debug)]
            struct ThreadPoolParams {
                name: String,
                num_threads: usize,
            }

            let params: ThreadPoolParams = lua.from_value(params)?;
            let num_threads = AtomicUsize::new(params.num_threads);

            if !config::is_validating() {
                // Create the runtime. We don't need to hold on
                // to it here, as it will be kept alive in the
                // runtimes map in that crate
                let _runtime = kumo_server_runtime::Runtime::new(
                    &params.name,
                    |_| params.num_threads,
                    &num_threads,
                )
                .map_err(any_err)?;
            }

            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "validation_failed",
        lua.create_function(|_, ()| {
            config::set_validation_failed();
            Ok(())
        })?,
    )?;

    kumo_mod.set(
        "enable_memory_callstack_tracking",
        lua.create_function(|_, enable: bool| {
            kumo_server_memory::set_tracking_callstacks(enable);
            Ok(())
        })?,
    )?;

    // This function is intended for debugging and testing purposes only.
    // It is potentially very expensive on a production system with many
    // thousands of queues.
    kumo_mod.set(
        "prometheus_metrics",
        lua.create_async_function(|lua, ()| async move {
            use tokio_stream::StreamExt;
            let mut json_text = String::new();
            let mut stream = kumo_prometheus::registry::Registry::stream_json();
            while let Some(text) = stream.next().await {
                json_text.push_str(&text);
            }
            let value: serde_json::Value = serde_json::from_str(&json_text).map_err(any_err)?;
            lua.to_value_with(&value, serialize_options())
        })?,
    )?;

    Ok(())
}
