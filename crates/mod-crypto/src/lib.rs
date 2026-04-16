use anyhow::anyhow;
use aws_lc_rs::cipher::{
    AES_128, AES_256, AES_CBC_IV_LEN, DecryptionContext, PaddedBlockDecryptingKey,
    PaddedBlockEncryptingKey, UnboundCipherKey,
};
use config::{any_err, from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use mlua::{Lua, Value};
use mod_digest::BinaryResult;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct AesParams {
    pub algorithm: AesAlgo,
    pub key: Vec<u8>,
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub enum AesAlgo {
    Ecb,
    Cbc,
}

#[derive(Deserialize, Clone, Debug)]
pub struct KeyConfig {
    pub key: KeySource,
}

fn make_cipher_key(bytes: &[u8]) -> anyhow::Result<UnboundCipherKey> {
    match bytes.len() {
        16 => Ok(UnboundCipherKey::new(&AES_128, bytes)?),
        32 => Ok(UnboundCipherKey::new(&AES_256, bytes)?),
        _ => anyhow::bail!("Key length must be 16 or 32 bytes"),
    }
}

fn make_enc_key(bytes: &[u8], algorithm: AesAlgo) -> anyhow::Result<PaddedBlockEncryptingKey> {
    let key = make_cipher_key(bytes)?;
    match algorithm {
        AesAlgo::Ecb => Ok(PaddedBlockEncryptingKey::ecb_pkcs7(key)?),
        AesAlgo::Cbc => Ok(PaddedBlockEncryptingKey::cbc_pkcs7(key)?),
    }
}

fn make_dec_key(bytes: &[u8], algorithm: AesAlgo) -> anyhow::Result<PaddedBlockDecryptingKey> {
    let key = make_cipher_key(bytes)?;
    match algorithm {
        AesAlgo::Ecb => Ok(PaddedBlockDecryptingKey::ecb_pkcs7(key)?),
        AesAlgo::Cbc => Ok(PaddedBlockDecryptingKey::cbc_pkcs7(key)?),
    }
}

fn aes_encrypt_block(plaintext: &[u8], params: AesParams) -> anyhow::Result<Vec<u8>> {
    let mut buf_ciphertext = plaintext.to_vec();
    let enc_key = make_enc_key(&params.key, params.algorithm)?;

    match params.algorithm {
        AesAlgo::Ecb => {
            // don't return IV vector for ecb
            enc_key.encrypt(&mut buf_ciphertext)?;
            Ok(buf_ciphertext)
        }
        AesAlgo::Cbc => {
            // context contain IV vector
            let context = enc_key.encrypt(&mut buf_ciphertext)?;

            match context {
                DecryptionContext::Iv128(iv) => {
                    let mut result = iv.as_ref().to_vec();
                    // Append the actual ciphertext after the IV
                    result.extend_from_slice(&buf_ciphertext);
                    Ok(result)
                }
                unsupported => anyhow::bail!(
                    "Unexpected IV context {unsupported:?} for encrypting in CBC mode"
                ),
            }
        }
    }
}

fn aes_decrypt_block(ciphertext_buf: &[u8], params: AesParams) -> anyhow::Result<BinaryResult> {
    let mut in_out_buffer = ciphertext_buf.to_vec();

    let dec_key = make_dec_key(&params.key, params.algorithm)?;
    match params.algorithm {
        AesAlgo::Ecb => match dec_key.decrypt(&mut in_out_buffer, DecryptionContext::None) {
            Ok(plaintext) => Ok(BinaryResult(plaintext.to_vec())),
            Err(e) => Err(anyhow!("Decryption failed with AES ECB mode: {}", e)),
        },

        AesAlgo::Cbc => {
            // CBC expects the IV to be prepended to the ciphertext.
            // Split into IV and actual ciphertext
            let Some((iv_bytes, actual_ciphertext)) =
                ciphertext_buf.split_at_checked(AES_CBC_IV_LEN)
            else {
                anyhow::bail!(
                    "ciphertext must be prefixed by iv with len at least {AES_CBC_IV_LEN}"
                );
            };

            // Create a mutable buffer for decryption
            let mut decrypt_buffer = actual_ciphertext.to_vec();
            let plaintext_slice = dec_key.decrypt(
                &mut decrypt_buffer,
                DecryptionContext::Iv128(iv_bytes.try_into()?),
            )?;
            Ok(BinaryResult(plaintext_slice.to_vec()))
        }
    }
}
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;
    crypto.set(
        "aes_encrypt_block",
        lua.create_async_function(
            |lua, (algorithm, data, config): (Value, mlua::String, Value)| async move {
                let algorithm: AesAlgo = from_lua_value(&lua, algorithm)?;

                let config: KeyConfig = from_lua_value(&lua, config)?;
                let key = config.key.get().await.map_err(any_err)?;
                let p = AesParams { key, algorithm };

                let plaintext_bytes = data.as_bytes();
                let result = aes_encrypt_block(&plaintext_bytes, p).map_err(any_err)?;
                lua.create_string(&result)
            },
        )?,
    )?;

    crypto.set(
        "aes_decrypt_block",
        lua.create_async_function(
            |lua, (algorithm, data, config): (Value, mlua::String, Value)| async move {
                let algorithm: AesAlgo = from_lua_value(&lua, algorithm)?;

                let config: KeyConfig = from_lua_value(&lua, config)?;
                let key = config.key.get().await.map_err(any_err)?;
                let p = AesParams { key, algorithm };

                let ciphertext_bytes = data.as_bytes();
                aes_decrypt_block(&ciphertext_bytes, p).map_err(any_err)
            },
        )?,
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
            // Example plaintext
            let plaintext = "helloword-from-the-sun";
            // Hex for a 128-bit AES key
            let key_hex = "2b7e151628aed2a6abf7158809cf4f3c";

            let key = hex::decode(key_hex)?;
            let params = AesParams {
                key,
                algorithm: AesAlgo::Ecb,
            };

            let ciphertext = aes_encrypt_block(plaintext.as_bytes(), params.clone())?;
            let decrypted_text = aes_decrypt_block(ciphertext.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            assert_eq!(decrypted_string, plaintext);

            Ok(())
        }
    }

    mod cbc_tests {
        use super::*;
        #[test]
        fn encrypt_decrypt_aes128_cbc_hex() -> Result<()> {
            let plaintext = "This is a secret message to be encrypted using AES-128 CBC mode.";
            // AES-128 key
            let key_hex = "2b7e151628aed2a6abf7158809cf4f3c";
            let key = hex::decode(key_hex)?;

            let params = AesParams {
                key,
                algorithm: AesAlgo::Cbc,
            };

            let ciphertext_with_iv = aes_encrypt_block(plaintext.as_bytes(), params.clone())?;

            assert!(
                ciphertext_with_iv.len() > AES_CBC_IV_LEN,
                "Ciphertext should contain IV + data"
            );

            let decrypted_text = aes_decrypt_block(ciphertext_with_iv.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            assert_eq!(decrypted_string, plaintext);

            Ok(())
        }

        #[test]
        fn encrypt_decrypt_aes256_cbc_hex() -> Result<()> {
            let plaintext = "Another secret message, but this one is for AES-256 CBC.";
            // AES-256 key
            let key_hex = "603deb1015ca71be2b73aef0857d7781a5b6b8e5b62c65e9f1f63b7ee7ec6f2f";
            let key = hex::decode(key_hex)?;

            let params = AesParams {
                key,
                algorithm: AesAlgo::Cbc,
            };

            let ciphertext_with_iv = aes_encrypt_block(plaintext.as_bytes(), params.clone())?;

            assert!(
                ciphertext_with_iv.len() > AES_CBC_IV_LEN,
                "Ciphertext should contain IV + data"
            );

            let decrypted_text = aes_decrypt_block(ciphertext_with_iv.as_slice(), params.clone())?;
            let decrypted_string = String::from_utf8(decrypted_text.0)?;
            assert_eq!(decrypted_string, plaintext);

            Ok(())
        }
    }
}
