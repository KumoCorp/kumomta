use config::{any_err, from_lua_value, get_or_create_module};
use kumo_server_common::http_server::HttpListenerParams;
use kumo_server_runtime::get_main_runtime;
use mlua::{Lua, Value};

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let tsa_mod = get_or_create_module(lua, "tsa")?;

    tsa_mod.set(
        "start_http_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: HttpListenerParams = from_lua_value(&lua, params)?;
            params
                .start(crate::http_server::make_router(), Some(get_main_runtime()))
                .await
                .map_err(any_err)?;
            Ok(())
        })?,
    )?;

    tsa_mod.set(
        "configure_tsa_db_path",
        lua.create_function(|_lua, file_name: String| {
            *crate::http_server::DB_PATH.lock().unwrap() = file_name;
            Ok(())
        })?,
    )?;

    Ok(())
}
