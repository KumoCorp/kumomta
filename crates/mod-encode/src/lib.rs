use config::{any_err, get_or_create_sub_module};
use data_encoding::{
    Encoding, BASE32, BASE32HEX, BASE32HEX_NOPAD, BASE32_NOPAD, BASE64, BASE64URL, BASE64URL_NOPAD,
    BASE64_NOPAD, HEXLOWER,
};
use mlua::{Lua, Value};

/// data_encoding is very strict when considering padding, making it
/// incompatible with a number of base64 encoders that apply excess
/// padding in certain situations.
/// This decode wrapper considers whether the encoder allows padding,
/// and if so, speculatively removes any trailing padding bytes from
/// the string and instead uses the no-padding variant of the encoder
/// specification in order to avoid triggering any length/padding
/// checks inside the crate.
fn decode(enc: &Encoding, data: &[u8]) -> mlua::Result<Vec<u8>> {
    let mut spec = enc.specification();
    if let Some(pad_char) = spec.padding {
        let padding_bytes = [pad_char as u8];
        let mut stripped = data;
        while let Some(s) = stripped.strip_suffix(&padding_bytes) {
            stripped = s;
        }
        spec.padding.take();
        return spec
            .encoding()
            .map_err(any_err)?
            .decode(stripped)
            .map_err(any_err);
    }
    enc.decode(data).map_err(any_err)
}

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
                let bytes = decode(&enc, data.as_bytes())?;
                lua.create_string(&bytes)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn test_decode_padding() {
    assert_eq!(
        std::str::from_utf8(&decode(&BASE64, b"MmVtYWlsLmxvZwAuY3N2").unwrap()).unwrap(),
        "2email.log\0.csv"
    );
    assert_eq!(
        std::str::from_utf8(&decode(&BASE64, b"MmVtYWlsLmxvZwAuY3N2=").unwrap()).unwrap(),
        "2email.log\0.csv"
    );
    assert_eq!(
        std::str::from_utf8(&decode(&BASE64, b"MmVtYWlsLmxvZwAuY3N2==").unwrap()).unwrap(),
        "2email.log\0.csv"
    );
    assert_eq!(
        std::str::from_utf8(&decode(&BASE64, b"MmVtYWlsLmxvZwAuY3N2===").unwrap()).unwrap(),
        "2email.log\0.csv"
    );
}
