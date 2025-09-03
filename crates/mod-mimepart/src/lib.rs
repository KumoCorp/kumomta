pub use crate::mimepart::PartRef;
use config::{SerdeWrappedValue, any_err, get_or_create_sub_module};
use mailparsing::{AttachmentOptions, MimePart, SharedString};
use mlua::{Lua, UserDataRef};

pub mod builder;
pub mod headers;
pub mod mimepart;

fn new_text_part(_: &Lua, (content_type, content): (String, String)) -> mlua::Result<PartRef> {
    let part = MimePart::new_text(&content_type, &content).map_err(any_err)?;
    Ok(PartRef::new(part))
}

fn new_text_plain(_: &Lua, content: String) -> mlua::Result<PartRef> {
    let part = MimePart::new_text_plain(&content).map_err(any_err)?;
    Ok(PartRef::new(part))
}

fn new_html(_: &Lua, content: String) -> mlua::Result<PartRef> {
    let part = MimePart::new_html(&content).map_err(any_err)?;
    Ok(PartRef::new(part))
}

fn new_binary(
    _: &Lua,
    (content_type, content, options): (
        String,
        mlua::String,
        Option<SerdeWrappedValue<AttachmentOptions>>,
    ),
) -> mlua::Result<PartRef> {
    let part = MimePart::new_binary(
        &content_type,
        content.as_bytes().as_ref(),
        options.as_deref(),
    )
    .map_err(any_err)?;
    Ok(PartRef::new(part))
}

fn new_multipart(
    _: &Lua,
    (content_type, parts, boundary): (String, Vec<UserDataRef<PartRef>>, Option<String>),
) -> mlua::Result<PartRef> {
    let mut child_parts = vec![];
    for p in parts {
        child_parts.push(p.resolve().map_err(any_err)?.to_owned());
    }

    let part = MimePart::new_multipart(&content_type, child_parts, boundary.as_deref())
        .map_err(any_err)?;
    Ok(PartRef::new(part))
}

fn parse_eml(_: &Lua, eml_contents: String) -> mlua::Result<PartRef> {
    let eml_contents: SharedString = eml_contents.into();
    let part = MimePart::parse(eml_contents).map_err(any_err)?;
    Ok(PartRef::new(part))
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_sub_module(lua, "mimepart")?;
    kumo_mod.set("parse", lua.create_function(parse_eml)?)?;
    kumo_mod.set("new_binary", lua.create_function(new_binary)?)?;
    kumo_mod.set("new_html", lua.create_function(new_html)?)?;
    kumo_mod.set("new_multipart", lua.create_function(new_multipart)?)?;
    kumo_mod.set("new_text", lua.create_function(new_text_part)?)?;
    kumo_mod.set("new_text_plain", lua.create_function(new_text_plain)?)?;
    kumo_mod.set(
        "builder",
        lua.create_function(|_lua, ()| Ok(crate::builder::Builder::new()))?,
    )?;
    Ok(())
}
