use anyhow::anyhow;
use aws_lc_rs::cipher::{
    AES_128, AES_256, DecryptionContext, PaddedBlockDecryptingKey, PaddedBlockEncryptingKey,
    UnboundCipherKey,
};
use data_encoding::{
    BASE32, BASE32HEX, BASE32HEX_NOPAD, BASE32_NOPAD, BASE64, BASE64URL, BASE64URL_NOPAD,
    BASE64_NOPAD, HEXLOWER,
};
use config::{any_err, from_lua_value, get_or_create_sub_module};
use mlua::prelude::LuaUserData;
use mlua::{Lua, MetaMethod, UserDataFields, UserDataMethods, Value, Error as LuaError};
use data_loader::KeySource;
use hex::decode;
use std::str;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct AesParams {
    pub algorithm: AesAlgo,
    pub key: AesKey,
}

#[derive(Deserialize, Clone, Debug)]
pub enum AesAlgo {
    Ecb(),
    Cbc(),
}

#[derive(Deserialize, Clone, Debug)]
pub struct LuaCfg {
    pub key: KeySource,
    pub value: String,
    pub algorithm: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LuaCfgDecrypt {
    pub key: KeySource,
    pub value: Vec<u8>,
    pub algorithm: String,
}

#[derive(Deserialize, Clone, Debug)]
pub enum AesKey {
    Aes128([u8; 16]),
    Aes256([u8; 32]),
}

fn is_hex(vec: &[u8]) -> bool {
    let hex_str = String::from_utf8_lossy(vec);
    hex_str.chars().all(|c| c.is_digit(16)) && hex_str.len() % 2 == 0
}

impl AesKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let decoded_bytes = if is_hex(bytes) {
            decode(bytes).map_err(|e| format!("Failed to decode hex: {}", e))?
        } else {
            bytes.to_vec()
        };
        match decoded_bytes.len() {
            16 => Ok(AesKey::Aes128(decoded_bytes.try_into().unwrap())),
            32 => Ok(AesKey::Aes256(decoded_bytes.try_into().unwrap())),
            _ => Err("Key length must be 16 or 32 bytes".to_string()),
        }
    }
}

fn aes_encrypt_block(plaintext: &str, params: AesParams) -> Result<Vec<u8>, anyhow::Error> {
    let mut buf_ciphertext = plaintext.as_bytes().to_vec();
    let enc_key: PaddedBlockEncryptingKey;

    match params.algorithm {
        AesAlgo::Ecb() => {
            enc_key = match params.key {
                AesKey::Aes128(k) => {
                    let key = UnboundCipherKey::new(&AES_128, &k)?;
                    PaddedBlockEncryptingKey::ecb_pkcs7(key)?
                }
                AesKey::Aes256(k) => {
                    let key = UnboundCipherKey::new(&AES_256, &k)?;
                    PaddedBlockEncryptingKey::ecb_pkcs7(key)?
                }
            };
            // don't return IV vector for ecb
            enc_key.encrypt(&mut buf_ciphertext)?;
            Ok(buf_ciphertext)
        }
        AesAlgo::Cbc() => {
            enc_key = match params.key {
                AesKey::Aes128(k) => {
                    let key = UnboundCipherKey::new(&AES_128, &k)?;
                    PaddedBlockEncryptingKey::cbc_pkcs7(key)?
                }
                AesKey::Aes256(k) => {
                    let key = UnboundCipherKey::new(&AES_256, &k)?;
                    PaddedBlockEncryptingKey::cbc_pkcs7(key)?
                }
            };

            // context contain IV vector
            let context = enc_key.encrypt(&mut buf_ciphertext)?;

            match context {
                DecryptionContext::Iv128(iv) => {
                    let mut result = iv.as_ref().to_vec();
                    result.extend_from_slice(&buf_ciphertext); // Append the actual ciphertext after the IV
                    Ok(result)
                }
                _ => Err(anyhow!("Unexpected IV context for encrypting in CBC mode")),
            }
        }
    }
}


struct DecryptResult(Vec<u8>);
impl LuaUserData for DecryptResult {
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

fn aes_decrypt_block(ciphertext_buf: &[u8], params: AesParams) -> anyhow::Result<DecryptResult>  {
    let mut in_out_buffer = ciphertext_buf.to_vec();
    let dec_key: PaddedBlockDecryptingKey;

    match params.algorithm {
        AesAlgo::Ecb() => {
            dec_key = match params.key {
                AesKey::Aes128(k) => {
                    let key = UnboundCipherKey::new(&AES_128, &k)?;
                    PaddedBlockDecryptingKey::ecb_pkcs7(key)?
                }
                AesKey::Aes256(k) => {
                    let key = UnboundCipherKey::new(&AES_256, &k)?;
                    PaddedBlockDecryptingKey::ecb_pkcs7(key)?
                }
            };

            match dec_key.decrypt(&mut in_out_buffer, DecryptionContext::None) {
                //     Ok(DigestResult(digest.as_ref().to_vec()))
                Ok(plaintext) => Ok(DecryptResult(plaintext.to_vec())),
                Err(_) => Err(anyhow!("Decryption failed with AES key, mode ecb block")),
            }
        }

        AesAlgo::Cbc() => {
            dec_key = match params.key {
                AesKey::Aes128(k) => {
                    let key = UnboundCipherKey::new(&AES_128, &k)?;
                    PaddedBlockDecryptingKey::cbc_pkcs7(key)?
                }
                AesKey::Aes256(k) => {
                    let key = UnboundCipherKey::new(&AES_256, &k)?;
                    PaddedBlockDecryptingKey::cbc_pkcs7(key)?
                }
            };

            // CBC expects the IV to be prepended to the ciphertext.
            if ciphertext_buf.len() < 16 {
                return Err(anyhow!(
                    "Ciphertext is too short to contain an IV (expected at least 16 bytes for IV)"
                ));
            }

            let (iv_bytes, actual_ciphertext) = ciphertext_buf.split_at(16); // Split into IV and actual ciphertext
            let iv: [u8; 16] = iv_bytes
                .try_into()
                .map_err(|_| anyhow!("IV length is incorrect, expected 16 bytes"))?;

            let mut decrypt_buffer = actual_ciphertext.to_vec(); // Create a mutable buffer for decryption
            let plaintext_slice =
                dec_key.decrypt(&mut decrypt_buffer, DecryptionContext::Iv128(iv.into()))?;
            Ok(DecryptResult(plaintext_slice.to_vec()))
        }
    }
}
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;
    crypto.set(
        "aes_encrypt_block",
        lua.create_async_function(|lua, params: Value| async move {
            let params: LuaCfg = from_lua_value(&lua, params)?;
            let aes_k =
                params.key.get().await.map_err(|e| {
                    LuaError::external(format!("key: {:?} failed: {}", params.key, e))
                })?;
            let aes_key = AesKey::from_bytes(&aes_k).map_err(any_err)?;

            let algo = match params.algorithm.to_lowercase().as_str() {
                "cbc" => AesAlgo::Cbc(),
                "ecb" => AesAlgo::Ecb(),
                _ => {
                    return Err(LuaError::external(format!(
                        "Invalid algorithm '{}'. Expected 'cbc' or 'ecb'",
                        params.algorithm
                    )));
                }
            };
            let p = AesParams {
                key: aes_key,
                algorithm: algo,
            };
            let result = aes_encrypt_block(&params.value, p).map_err(any_err)?;
            Ok(result)
        })?,
    )?;

    crypto.set(
        "aes_decrypt_block",
        lua.create_async_function(|lua, params: Value| async move {
            let params: LuaCfgDecrypt = from_lua_value(&lua, params)?;
            let aes_k =
                params.key.get().await.map_err(|e| {
                    LuaError::external(format!("key: {:?} failed: {}", params.key, e))
                })?;

            let algo = match params.algorithm.to_lowercase().as_str() {
                "cbc" => AesAlgo::Cbc(),
                "ecb" => AesAlgo::Ecb(),
                _ => {
                    return Err(LuaError::external(format!(
                        "Invalid algorithm '{}'. Expected 'cbc' or 'ecb'",
                        params.algorithm
                    )));
                }
            };
            let aes_key = AesKey::from_bytes(&aes_k).map_err(any_err)?;
            let p = AesParams {
                key: aes_key,
                algorithm: algo,
            };
            let result = aes_decrypt_block(&params.value, p).map_err(any_err)?;
            Ok(result)
        })?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use hex;

    mod ecb_tests {
        use super::*;
        #[test]
        fn encrypt_decrypt_aes128_ecb_hex() -> Result<()> {
            let plaintext = "helloword-from-the-sun"; // Example plaintext
            let key_hex = "2b7e151628aed2a6abf7158809cf4f3c"; // Hex for a 128-bit AES key

            let key = hex::decode(key_hex)?;
            let params = AesParams {
                key: AesKey::Aes128(key.as_slice().try_into().unwrap()),
                algorithm: AesAlgo::Ecb(),
            };

            let ciphertext = aes_encrypt_block(plaintext, params.clone())?;
            let decrypted_text = aes_decrypt_block(ciphertext.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;

            // Trim PKCS7 padding before assertion
            let trimmed_string = decrypted_string.trim_end_matches(|c| c as u8 <= 16);
            assert_eq!(trimmed_string, plaintext);

            Ok(())
        }
        #[test]
        fn encrypt_decrypt_aes256_ecb_hex() -> Result<()> {
            let plaintext = "helloword-from-the-water";
            let key_hex = "603deb1015ca71be2b73aef0857d7781a5b6b8e5b62c65e9f1f63b7ee7ec6f2f";
            let key = hex::decode(key_hex)?;
            let params = AesParams {
                key: AesKey::Aes256(key.as_slice().try_into().unwrap()),
                algorithm: AesAlgo::Ecb(),
            };
            let ciphertext = aes_encrypt_block(plaintext, params.clone())?;
            let decrypted_text = aes_decrypt_block(ciphertext.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            
            // Trim PKCS7 padding before assertion
            let trimmed_string = decrypted_string.trim_end_matches(|c| c as u8 <= 16);
            assert_eq!(trimmed_string, plaintext);

            Ok(())
        }
    }

    mod cbc_tests {
        use super::*;
        #[test]
        fn encrypt_decrypt_aes128_cbc_hex() -> Result<()> {
            let plaintext = "This is a secret message to be encrypted using AES-128 CBC mode.";
            let key_hex = "2b7e151628aed2a6abf7158809cf4f3c"; // AES-128 key
            let key = hex::decode(key_hex)?;

            let params = AesParams {
                key: AesKey::Aes128(key.as_slice().try_into().unwrap()),
                algorithm: AesAlgo::Cbc(),
            };

            let ciphertext_with_iv = aes_encrypt_block(plaintext, params.clone())?;

            assert!(
                ciphertext_with_iv.len() > 16,
                "Ciphertext should contain IV + data"
            );

            let decrypted_text = aes_decrypt_block(ciphertext_with_iv.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            
            // Trim PKCS7 padding before assertion
            let trimmed_string = decrypted_string.trim_end_matches(|c| c as u8 <= 16);
            assert_eq!(trimmed_string, plaintext);

            Ok(())
        }

        #[test]
        fn encrypt_decrypt_aes256_cbc_hex() -> Result<()> {
            let plaintext = "Another secret message, but this one is for AES-256 CBC.";
            let key_hex = "603deb1015ca71be2b73aef0857d7781a5b6b8e5b62c65e9f1f63b7ee7ec6f2f"; // AES-256 key
            let key = hex::decode(key_hex)?;

            let params = AesParams {
                key: AesKey::Aes256(key.as_slice().try_into().unwrap()),
                algorithm: AesAlgo::Cbc(),
            };

            let ciphertext_with_iv = aes_encrypt_block(plaintext, params.clone())?;

            assert!(
                ciphertext_with_iv.len() > 16,
                "Ciphertext should contain IV + data"
            );

            let decrypted_text = aes_decrypt_block(ciphertext_with_iv.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            
            // Trim PKCS7 padding before assertion
            let trimmed_string = decrypted_string.trim_end_matches(|c| c as u8 <= 16);
            assert_eq!(trimmed_string, plaintext);

            Ok(())
        }
    }
}