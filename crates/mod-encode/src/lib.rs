use config::{any_err, get_or_create_sub_module};
use data_encoding::{
    BASE32, BASE32HEX, BASE32HEX_NOPAD, BASE32_NOPAD, BASE64, BASE64URL, BASE64URL_NOPAD,
    BASE64_NOPAD, HEXLOWER,
};
use mlua::{Lua, Value};

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let digest_mod = get_or_create_sub_module(lua, "encode")?;

    for (name, enc) in [
        ("base32", BASE32),
        ("base32hex", BASE32HEX),
        ("base32hex_nopad", BASE32HEX_NOPAD),
        ("base32_nopad", BASE32_NOPAD),
        ("base64", BASE64),
        ("base64url", BASE64URL),
        ("base64url_nopad", BASE64URL_NOPAD),
        ("base64_nopad", BASE64_NOPAD),
        ("hex", HEXLOWER),
    ] {
        let encoder = enc.clone();
        digest_mod.set(
            format!("{name}_encode"),
            lua.create_function(move |_, data: mlua::Value| match data {
                Value::String(s) => Ok(encoder.encode(&s.as_bytes())),
                _ => Err(mlua::Error::external(
                    "parameter must be a string".to_string(),
                )),
            })?,
        )?;
        digest_mod.set(
            format!("{name}_decode"),
            lua.create_function(move |lua, data: String| {
                let bytes = enc.decode(data.as_bytes()).map_err(any_err)?;
                lua.create_string(&bytes)
            })?,
        )?;
    }
    Ok(())
}
