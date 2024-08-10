use config::{any_err, get_or_create_sub_module};
use mlua::{Lua, LuaSerdeExt};

mod amqprs_client;
mod lapin_client;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let amqp_mod = get_or_create_sub_module(lua, "amqp")?;

    amqp_mod.set(
        "build_client",
        lua.create_async_function(|_, uri: String| async move {
            lapin_client::build_client(uri).await.map_err(any_err)
        })?,
    )?;

    amqp_mod.set(
        "basic_publish",
        lua.create_async_function(|lua, params: mlua::Value| async move {
            let params: amqprs_client::PublishParams = lua.from_value(params)?;
            amqprs_client::publish(params).await.map_err(any_err)
        })?,
    )?;

    Ok(())
}
