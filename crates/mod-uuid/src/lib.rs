use config::{any_err, get_or_create_sub_module};
use mlua::{Lua, MetaMethod, UserData, UserDataFields, UserDataMethods};
use uuid::Uuid;

struct WrappedUuid(Uuid);

impl UserData for WrappedUuid {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("hyphenated", |_, this| {
            Ok(format!("{}", this.0.as_hyphenated()))
        });
        fields.add_field_method_get("simple", |_, this| Ok(format!("{}", this.0.as_simple())));
        fields.add_field_method_get("braced", |_, this| Ok(format!("{}", this.0.as_braced())));
        fields.add_field_method_get("urn", |_, this| Ok(format!("{}", this.0.as_urn())));
        fields.add_field_method_get("bytes", |lua, this| lua.create_string(this.0.as_bytes()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(format!("{}", this.0.as_hyphenated()))
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let uuid_mod = get_or_create_sub_module(lua, "uuid")?;

    uuid_mod.set(
        "parse",
        lua.create_function(|_, s: String| Ok(WrappedUuid(Uuid::try_parse(&s).map_err(any_err)?)))?,
    )?;

    uuid_mod.set(
        "new_v1",
        lua.create_function(|_, _: ()| Ok(WrappedUuid(uuid_helper::now_v1())))?,
    )?;
    uuid_mod.set(
        "new_v4",
        lua.create_function(|_, _: ()| Ok(WrappedUuid(Uuid::new_v4())))?,
    )?;
    uuid_mod.set(
        "new_v6",
        lua.create_function(|_, _: ()| {
            Ok(WrappedUuid(Uuid::now_v6(uuid_helper::get_mac_address())))
        })?,
    )?;
    uuid_mod.set(
        "new_v7",
        lua.create_function(|_, _: ()| Ok(WrappedUuid(Uuid::now_v7())))?,
    )?;

    Ok(())
}
