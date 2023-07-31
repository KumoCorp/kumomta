use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use mlua::{Function, Lua, LuaSerdeExt, Value};
use mod_redis::RedisConnKey;

pub mod diagnostic_logging;
pub mod http_server;
pub mod nodeid;
pub mod panic;
pub mod start;
pub mod tls_helpers;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    for func in [
        mod_redis::register,
        data_loader::register,
        domain_map::register,
        mod_amqp::register,
        mod_http::register,
        mod_sqlite::register,
        mod_dns_resolver::register,
        mod_memoize::register,
        kumo_api_types::shaping::register,
    ] {
        func(lua)?;
    }

    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "on",
        lua.create_function(move |lua, (name, func): (String, Function)| {
            let decorated_name = format!("kumomta-on-{}", name);

            let existing: Value = lua.named_registry_value(&decorated_name)?;
            match existing {
                Value::Nil => {}
                Value::Function(func) => {
                    let info = func.info();
                    let src = String::from_utf8_lossy(
                        info.source.as_ref().map(|v| v.as_slice()).unwrap_or(b"?"),
                    );
                    let line = info.line_defined;
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
        "configure_redis_throttles",
        lua.create_async_function(|lua, params: Value| async move {
            let key: RedisConnKey = from_lua_value(lua, params)?;
            let conn = key.open().await.map_err(any_err)?;
            throttle::use_redis(conn).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "toml_load",
        lua.create_async_function(|lua, file_name: String| async move {
            let data = tokio::fs::read_to_string(&file_name)
                .await
                .with_context(|| format!("reading file {file_name}"))
                .map_err(any_err)?;

            let obj: toml::Value = toml::from_str(&data)
                .with_context(|| format!("parsing {file_name} as toml"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "toml_parse",
        lua.create_function(move |lua, toml: String| {
            let obj: toml::Value = toml::from_str(&toml)
                .with_context(|| format!("parsing {toml} as toml"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "toml_encode",
        lua.create_function(move |_lua, value: Value| toml::to_string(&value).map_err(any_err))?,
    )?;

    kumo_mod.set(
        "toml_encode_pretty",
        lua.create_function(move |_lua, value: Value| {
            toml::to_string_pretty(&value).map_err(any_err)
        })?,
    )?;

    kumo_mod.set(
        "json_load",
        lua.create_async_function(|lua, file_name: String| async move {
            let data = tokio::fs::read(&file_name)
                .await
                .with_context(|| format!("reading file {file_name}"))
                .map_err(any_err)?;

            let stripped = json_comments::StripComments::new(&*data);

            let obj: serde_json::Value = serde_json::from_reader(stripped)
                .with_context(|| format!("parsing {file_name} as json"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "json_parse",
        lua.create_async_function(|lua, text: String| async move {
            let stripped = json_comments::StripComments::new(text.as_bytes());
            let obj: serde_json::Value = serde_json::from_reader(stripped)
                .with_context(|| format!("parsing {text} as json"))
                .map_err(any_err)?;
            Ok(lua.to_value(&obj))
        })?,
    )?;

    kumo_mod.set(
        "json_encode",
        lua.create_async_function(|_, value: Value| async move {
            serde_json::to_string(&value).map_err(any_err)
        })?,
    )?;
    kumo_mod.set(
        "json_encode_pretty",
        lua.create_async_function(|_, value: Value| async move {
            serde_json::to_string_pretty(&value).map_err(any_err)
        })?,
    )?;

    Ok(())
}
