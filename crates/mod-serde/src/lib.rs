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
    serde_mod.set(
        "toml_encode_pretty_compact",
        lua.create_function(toml_encode_pretty_compact_lua)?,
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
    // This additional conversion to a json value causes any object types
    // to become backed by a BTreeMap rather than lua's randomly ordered
    // table type, so that the pretty print will show the keys in sorted order.
    // We only do that for pretty printing because we don't usually want
    // to bother with that additional overhead
    let value = serde_json::to_value(&value).map_err(any_err)?;
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

/// Render a value as pretty TOML with:
///
/// - Keys sorted alphabetically at every nesting level.
/// - Empty tables emitted inline as `{}` rather than as a `[name]`
///   header. Empty arrays already render inline as `[]` via the
///   default toml serializer, so no special handling is needed.
///
/// Useful for human-readable diagnostic output where stable
/// scan-friendly ordering matters and empty-table headers would
/// add visual noise.
pub fn toml_encode_pretty_compact<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    let initial = toml::to_string(value).context("serializing value as TOML")?;
    let mut doc: toml_edit::DocumentMut = initial
        .parse()
        .context("re-parsing TOML for format normalization")?;
    fn normalize(table: &mut toml_edit::Table) {
        // Collect keys of empty sub-tables and re-insert them as
        // inline empty tables via `insert()`, which establishes the
        // standard `key = {}` decor. Mutating the Item in place
        // would leave the original table-header decor on the key
        // (no `=` separator).
        let empty_keys: Vec<String> = table
            .iter()
            .filter_map(|(k, v)| match v {
                toml_edit::Item::Table(t) if t.is_empty() => Some(k.to_string()),
                _ => None,
            })
            .collect();
        for key in empty_keys {
            table.insert(
                &key,
                toml_edit::Item::Value(toml_edit::Value::InlineTable(
                    toml_edit::InlineTable::new(),
                )),
            );
        }

        // Recurse before sorting; sub-tables that remain (non-empty)
        // get their own inner sort.
        for (_, item) in table.iter_mut() {
            match item {
                toml_edit::Item::Table(t) => normalize(t),
                toml_edit::Item::ArrayOfTables(arr) => {
                    for t in arr.iter_mut() {
                        normalize(t);
                    }
                }
                _ => {}
            }
        }

        // Sort the value entries alphabetically. TOML grammar
        // requires sub-tables to follow value entries, so sub-tables
        // naturally trail the sorted values at this level.
        table.sort_values();
    }
    normalize(doc.as_table_mut());
    Ok(doc.to_string())
}

fn toml_encode_pretty_compact_lua(lua: &Lua, value: LuaValue) -> mlua::Result<String> {
    let value = materialize_to_lua_value(lua, value)?;
    toml_encode_pretty_compact(&value).map_err(any_err)
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
