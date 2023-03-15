use config::{any_err, get_or_create_sub_module};
use dns_resolver::MailExchanger;
use mlua::{Lua, LuaSerdeExt};

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dns_mod = get_or_create_sub_module(lua, "dns")?;

    dns_mod.set(
        "lookup_mx",
        lua.create_async_function(|lua, domain: String| async move {
            let mx = MailExchanger::resolve(&domain).await.map_err(any_err)?;
            Ok(lua.to_value(&*mx))
        })?,
    )?;

    Ok(())
}
