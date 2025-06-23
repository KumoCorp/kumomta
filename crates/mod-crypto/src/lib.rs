use config::get_or_create_sub_module;
use mlua::Lua;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;

       crypto.set(
        "hello",
        lua.create_function(|_, (name, age): (String, u8)| {
            println!("{} is {} years old!", name, age);
            Ok(())
        })?,
    )?;

    Ok(())
}
