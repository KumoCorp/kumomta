use config::{any_err, get_or_create_sub_module};
use crc32fast::Hasher;
use data_encoding::{
    BASE32, BASE32HEX, BASE32HEX_NOPAD, BASE32_NOPAD, BASE64, BASE64URL, BASE64URL_NOPAD,
    BASE64_NOPAD, HEXLOWER,
};
use mlua::prelude::LuaUserData;
use mlua::{Lua, MetaMethod, UserDataFields, UserDataMethods, Value, Variadic};
use ring::digest::*;

fn digest_recursive(value: &Value, ctx: &mut Context) -> anyhow::Result<()> {
    match value {
        Value::String(s) => {
            ctx.update(&s.as_bytes());
        }
        _ => {
            let json = serde_json::to_string(value)?;
            ctx.update(json.as_bytes());
        }
    }
    Ok(())
}

fn digest_helper(
    algorithm: &'static Algorithm,
    args: Variadic<Value>,
) -> anyhow::Result<DigestResult> {
    let mut ctx = Context::new(algorithm);
    for item in args.iter() {
        digest_recursive(item, &mut ctx)?;
    }
    let digest = ctx.finish();
    Ok(DigestResult(digest.as_ref().to_vec()))
}

struct DigestResult(Vec<u8>);

impl LuaUserData for DigestResult {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("hex", |_, this| Ok(HEXLOWER.encode(&this.0)));

        fields.add_field_method_get("base32", |_, this| Ok(BASE32.encode(&this.0)));
        fields.add_field_method_get("base32_nopad", |_, this| Ok(BASE32_NOPAD.encode(&this.0)));

        fields.add_field_method_get("base32hex", |_, this| Ok(BASE32HEX.encode(&this.0)));
        fields.add_field_method_get("base32hex_nopad", |_, this| {
            Ok(BASE32HEX_NOPAD.encode(&this.0))
        });

        fields.add_field_method_get("base64", |_, this| Ok(BASE64.encode(&this.0)));
        fields.add_field_method_get("base64_nopad", |_, this| Ok(BASE64_NOPAD.encode(&this.0)));

        fields.add_field_method_get("base64url", |_, this| Ok(BASE64URL.encode(&this.0)));
        fields.add_field_method_get("base64url_nopad", |_, this| {
            Ok(BASE64URL_NOPAD.encode(&this.0))
        });

        fields.add_field_method_get("bytes", |lua, this| lua.create_string(&this.0));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(HEXLOWER.encode(&this.0))
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let digest_mod = get_or_create_sub_module(lua, "digest")?;

    digest_mod.set(
        "sha1",
        lua.create_function(|_, args: Variadic<Value>| {
            digest_helper(&SHA1_FOR_LEGACY_USE_ONLY, args).map_err(any_err)
        })?,
    )?;
    digest_mod.set(
        "sha256",
        lua.create_function(|_, args: Variadic<Value>| {
            digest_helper(&SHA256, args).map_err(any_err)
        })?,
    )?;
    digest_mod.set(
        "sha384",
        lua.create_function(|_, args: Variadic<Value>| {
            digest_helper(&SHA384, args).map_err(any_err)
        })?,
    )?;
    digest_mod.set(
        "sha512",
        lua.create_function(|_, args: Variadic<Value>| {
            digest_helper(&SHA512, args).map_err(any_err)
        })?,
    )?;
    digest_mod.set(
        "sha512_256",
        lua.create_function(|_, args: Variadic<Value>| {
            digest_helper(&SHA512_256, args).map_err(any_err)
        })?,
    )?;
    digest_mod.set(
        "crc32",
        lua.create_function(|_, args: Variadic<Value>| {
            let mut hasher = Hasher::new();
            for item in args.iter() {
                match item {
                    Value::String(s) => {
                        hasher.update(&s.as_bytes());
                    }
                    _ => {
                        let json = serde_json::to_string(item).map_err(any_err)?;
                        hasher.update(json.as_bytes());
                    }
                }
            }
            let crc = hasher.finalize();
            Ok(DigestResult(crc.to_be_bytes().to_vec()))
        })?,
    )?;
    Ok(())
}
