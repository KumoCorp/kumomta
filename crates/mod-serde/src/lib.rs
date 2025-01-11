use anyhow::Context;
use config::{
    any_err, get_or_create_module, get_or_create_sub_module, materialize_to_lua_value,
    serialize_options,
};
use mlua::{Lua, LuaSerdeExt, Value as LuaValue};
use serde_json::Value as JValue;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let serde_mod = get_or_create_sub_module(lua, "serde")?;
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    serde_mod.set("json_load", lua.create_async_function(json_load)?)?;
    serde_mod.set("json_parse", lua.create_function(json_parse)?)?;
    serde_mod.set("json_encode", lua.create_function(json_encode)?)?;
    serde_mod.set(
        "json_encode_pretty",
        lua.create_function(json_encode_pretty)?,
    )?;

    serde_mod.set("toml_load", lua.create_async_function(toml_load)?)?;
    serde_mod.set("toml_parse", lua.create_function(toml_parse)?)?;
    serde_mod.set("toml_encode", lua.create_function(toml_encode)?)?;
    serde_mod.set(
        "toml_encode_pretty",
        lua.create_function(toml_encode_pretty)?,
    )?;

    serde_mod.set("yaml_load", lua.create_async_function(yaml_load)?)?;
    serde_mod.set("yaml_parse", lua.create_function(yaml_parse)?)?;
    serde_mod.set("yaml_encode", lua.create_function(yaml_encode)?)?;
    // Note there is no pretty encoder for yaml, because the default one is pretty already.
    // See https://github.com/dtolnay/serde-yaml/issues/226

    // Backwards compatibility
    kumo_mod.set("json_load", lua.create_async_function(json_load)?)?;
    kumo_mod.set("json_parse", lua.create_function(json_parse)?)?;
    kumo_mod.set("json_encode", lua.create_function(json_encode)?)?;
    kumo_mod.set(
        "json_encode_pretty",
        lua.create_function(json_encode_pretty)?,
    )?;

    kumo_mod.set("toml_load", lua.create_async_function(toml_load)?)?;
    kumo_mod.set("toml_parse", lua.create_function(toml_parse)?)?;
    kumo_mod.set("toml_encode", lua.create_function(toml_encode)?)?;
    kumo_mod.set(
        "toml_encode_pretty",
        lua.create_function(toml_encode_pretty)?,
    )?;

    Ok(())
}

async fn json_load(lua: Lua, file_name: String) -> mlua::Result<LuaValue> {
    let data = tokio::fs::read(&file_name)
        .await
        .with_context(|| format!("reading file {file_name}"))
        .map_err(any_err)?;

    let stripped = json_comments::StripComments::new(&*data);

    let obj: serde_json::Value = serde_json::from_reader(stripped)
        .with_context(|| format!("parsing {file_name} as json"))
        .map_err(any_err)?;
    lua.to_value_with(&obj, serialize_options())
}

fn json_parse(lua: &Lua, text: String) -> mlua::Result<LuaValue> {
    let stripped = json_comments::StripComments::new(text.as_bytes());
    let obj: serde_json::Value = serde_json::from_reader(stripped)
        .with_context(|| format!("parsing {text} as json"))
        .map_err(any_err)?;
    lua.to_value_with(&obj, serialize_options())
}

fn json_encode(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    serde_json::to_string(&value).map_err(any_err)
}

fn json_encode_pretty(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    serde_json::to_string_pretty(&value).map_err(any_err)
}

async fn toml_load(lua: Lua, file_name: String) -> mlua::Result<LuaValue> {
    let data = tokio::fs::read_to_string(&file_name)
        .await
        .with_context(|| format!("reading file {file_name}"))
        .map_err(any_err)?;

    let obj: toml::Value = toml::from_str(&data)
        .with_context(|| format!("parsing {file_name} as toml"))
        .map_err(any_err)?;
    lua.to_value_with(&obj, serialize_options())
}

fn toml_parse(lua: &Lua, toml: String) -> mlua::Result<LuaValue> {
    let obj: toml::Value = toml::from_str(&toml)
        .with_context(|| format!("parsing {toml} as toml"))
        .map_err(any_err)?;
    lua.to_value_with(&obj, serialize_options())
}

fn toml_encode(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    toml::to_string(&value).map_err(any_err)
}

fn toml_encode_pretty(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    toml::to_string_pretty(&value).map_err(any_err)
}

async fn yaml_load(lua: Lua, file_name: String) -> mlua::Result<LuaValue> {
    let data = tokio::fs::read(&file_name)
        .await
        .with_context(|| format!("reading file {file_name}"))
        .map_err(any_err)?;

    let value: JValue = serde_yaml::from_slice(&data)
        .with_context(|| format!("parsing {file_name} as yaml"))
        .map_err(any_err)?;
    lua.to_value_with(&value, serialize_options())
}

fn yaml_parse(lua: &Lua, text: String) -> mlua::Result<LuaValue> {
    let value: JValue = serde_yaml::from_str(&text)
        .with_context(|| format!("parsing {text} as yaml"))
        .map_err(any_err)?;
    lua.to_value_with(&value, serialize_options())
}

fn yaml_encode(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    serde_yaml::to_string(&value).map_err(any_err)
}
