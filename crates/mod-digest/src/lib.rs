use aws_lc_rs::digest::{Algorithm as DigestAlgo, Context as DigestContext};
use config::{any_err, from_lua_value, get_or_create_sub_module};
use crc32fast::Hasher;
use data_encoding::{
    BASE32, BASE32HEX, BASE32HEX_NOPAD, BASE32_NOPAD, BASE64, BASE64URL, BASE64URL_NOPAD,
    BASE64_NOPAD, HEXLOWER,
};
use data_loader::KeySource;
use mlua::prelude::LuaUserData;
use mlua::{Lua, MetaMethod, UserDataFields, UserDataMethods, Value, Variadic};

fn digest_recursive(value: &Value, ctx: &mut DigestContext) -> anyhow::Result<()> {
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
    algorithm: &'static DigestAlgo,
    args: Variadic<Value>,
) -> anyhow::Result<BinaryResult> {
    let mut ctx = DigestContext::new(algorithm);
    for item in args.iter() {
        digest_recursive(item, &mut ctx)?;
    }
    let digest = ctx.finish();
    Ok(BinaryResult(digest.as_ref().to_vec()))
}

pub struct BinaryResult(pub Vec<u8>);

impl LuaUserData for BinaryResult {
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

fn hmac_signer(
    algo: aws_lc_rs::hmac::Algorithm,
    key: &[u8],
    msg: &[u8],
) -> mlua::Result<BinaryResult> {
    use aws_lc_rs::hmac::Key;

    let key = Key::new(algo, key);
    let tag = aws_lc_rs::hmac::sign(&key, msg);

    Ok(BinaryResult(tag.as_ref().to_vec()))
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let digest_mod = get_or_create_sub_module(lua, "digest")?;

    macro_rules! digest {
        ($func_name:literal, $algo:ident) => {
            digest_mod.set(
                $func_name,
                lua.create_function(|_, args: Variadic<Value>| {
                    digest_helper(&aws_lc_rs::digest::$algo, args).map_err(any_err)
                })?,
            )?;
        };
    }

    macro_rules! hmac {
        ($func_name:literal, $algo:ident) => {
            digest_mod.set(
                concat!("hmac_", $func_name),
                lua.create_async_function(
                    |lua, (key, msg): (mlua::Value, mlua::String)| async move {
                        let key: KeySource = from_lua_value(&lua, key)?;
                        let key_bytes = key.get().await.map_err(any_err)?;

                        hmac_signer(aws_lc_rs::hmac::$algo, &key_bytes, &msg.as_bytes())
                    },
                )?,
            )?;
        };
    }

    digest!("sha1", SHA1_FOR_LEGACY_USE_ONLY);
    digest!("sha224", SHA224);
    digest!("sha256", SHA256);
    digest!("sha384", SHA384);
    digest!("sha3_256", SHA3_256);
    digest!("sha3_384", SHA3_384);
    digest!("sha3_512", SHA3_512);
    digest!("sha512", SHA512);
    digest!("sha512_256", SHA512_256);

    hmac!("sha1", HMAC_SHA1_FOR_LEGACY_USE_ONLY);
    hmac!("sha224", HMAC_SHA224);
    hmac!("sha256", HMAC_SHA256);
    hmac!("sha384", HMAC_SHA384);
    hmac!("sha512", HMAC_SHA512);

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
            Ok(BinaryResult(crc.to_be_bytes().to_vec()))
        })?,
    )?;
    Ok(())
}
