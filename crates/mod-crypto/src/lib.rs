use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use config::{from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use std::str;
use mlua::{Error as LuaError, Lua, Value};
use serde::Deserialize;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

use rand::RngCore;
use rand::rngs::OsRng;

fn generate_iv() -> [u8; 16] {
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut iv);
    iv
}

#[derive(Deserialize, Clone, Debug)]
pub struct AesParams {
    //  pub lua_key: Option<KeySource>,
    pub key: AesKey,
}
#[derive(Deserialize, Clone, Debug)]
pub struct LuaCfg {
    pub key: Option<KeySource>,
    pub value: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LuaCfgDecrypt {
    pub key: Option<KeySource>,
    pub decrypted: Vec<u8>,
}

#[derive(Deserialize, Clone, Debug)]
pub enum AesKey {
    Aes128([u8; 16]),
    Aes256([u8; 32]),
}

// we handle only bytes not hex stored keys
impl AesKey {
    // Associated function to create AesKey from raw bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        match bytes.len() {
            16 => {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(bytes);
                Ok(AesKey::Aes128(arr))
            }
            32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(bytes);
                Ok(AesKey::Aes256(arr))
            }
            _ => Err("Key length must be 16 or 32 bytes"),
        }
    }
}

fn aes_encrypt_cbc(plaintext: &str, params: AesParams) -> Result<Vec<u8>, &'static str> {
    let plaintext_bytes = plaintext.as_bytes();
    // Buffer must be big enough for padded plaintext
    //  -  for PKCS7 padding, max size = plaintext length + block size 16
    let mut buf = vec![0u8; plaintext.len() + 16];

    let iv = generate_iv();
    buf[..plaintext.len()].copy_from_slice(plaintext_bytes); // Copy plaintext to buffer
    match params.key {
        AesKey::Aes128(k) => {
            let cipher = Aes128CbcEnc::new((&k).into(), (&iv).into());
            match cipher.encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext_bytes.len()) {
                Ok(ct) => {
                    let mut output = iv.to_vec(); // IV is 16 bytes
                    output.extend_from_slice(ct);
                    Ok(output)
                }
                Err(_) => Err("Encryption failed or padding error"),
            }
        }
        AesKey::Aes256(k) => {
            let cipher = Aes256CbcEnc::new((&k).into(), (&iv).into());
            match cipher.encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext_bytes.len()) {
                Ok(ct) => {
                    let mut output = iv.to_vec(); // IV is 16 bytes
                    output.extend_from_slice(ct);
                    Ok(output)
                }
                Err(_) => Err("Encryption failed or padding error"),
            }
        }
    }
}

fn aes_decrypt_cbc(ciphertext_with_iv: &[u8], params: AesParams) -> Result<Vec<u8>, &'static str> {
    let (iv, ciphertext) = ciphertext_with_iv.split_at(16);
    let mut buf = ciphertext.to_vec();
    match params.key {
        AesKey::Aes128(k) => {
            let cipher = Aes128CbcDec::new((&k).into(), (iv).into());
            cipher
                .decrypt_padded_mut::<Pkcs7>(&mut buf)
                .map(|pt| pt.to_vec())
                .map_err(|_| "Decryption failed for AES-128")
        }
        AesKey::Aes256(k) => {
            let cipher = Aes256CbcDec::new((&k).into(), (iv).into());
            cipher
                .decrypt_padded_mut::<Pkcs7>(&mut buf)
                .map(|pt| pt.to_vec())
                .map_err(|_| "Decryption failed for AES-256")
        }
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;
    crypto.set(
        "aes_encrypt_cbc",
        lua.create_async_function(|lua, params: Value| async move {
            let params: LuaCfg = from_lua_value(&lua, params)?;

            let aes_k = match params.key {
                Some(key) => key
                    .get()
                    .await
                    .map_err(|e| LuaError::external(format!("key.get() failed: {}", e)))?,
                None => {
                    return Err(LuaError::external(
                        "No AES key was provided for encryption ",
                    ));
                }
            };

            let aes_key =
                AesKey::from_bytes(&aes_k).map_err(|e| LuaError::external(e.to_string()))?;
            let p = AesParams { key: aes_key };
            let result =
                aes_encrypt_cbc(&params.value, p).map_err(|e| LuaError::external(e.to_string()))?;
            Ok(result)
        })?,
    )?;
    crypto.set(
        "aes_decrypt_cbc",
        lua.create_async_function(|lua, params: Value| async move {
            let params: LuaCfgDecrypt = from_lua_value(&lua, params)?;
            let aes_k = match params.key {
                Some(key) => key
                    .get()
                    .await
                    .map_err(|e| LuaError::external(format!("key.get() failed: {}", e)))?,
                None => return Err(LuaError::external("No AES key was provided for decryption")),
            };

            let aes_key =
                AesKey::from_bytes(&aes_k).map_err(|e| LuaError::external(e.to_string()))?;

            let p = AesParams { key: aes_key };
            let result = aes_decrypt_cbc(&params.decrypted, p)
                .map_err(|e| LuaError::external(e.to_string()))?;
            Ok(result)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_encrypt_decrypt_aes128() {
        let key = AesKey::Aes128([0x11; 16]);
        let params = AesParams { key: key.clone() };

        let plaintext = "This is a test message for AES-128";
        let ciphertext = aes_encrypt_cbc(plaintext, params.clone()).expect("Encryption failed");
        let decrypted = aes_decrypt_cbc(&ciphertext, params).expect("Decryption failed");

        assert_eq!(decrypted, plaintext.as_bytes());
    }

    #[test]
    fn test_encrypt_decrypt_aes256() {
        let key = AesKey::Aes256([0x33; 32]);
        let params = AesParams { key: key.clone() };
        let plaintext = "This is a test message for AES-256 encryption";
        let ciphertext = aes_encrypt_cbc(plaintext, params.clone()).expect("Encryption failed");
        let decrypted = aes_decrypt_cbc(&ciphertext, params).expect("Decryption failed");
        assert_eq!(decrypted, plaintext.as_bytes());
    }

    #[test]
    fn test_decrypt_with_wrong_key_fails() {
        let correct_key = AesKey::Aes128([0x11; 16]);
        let wrong_key = AesKey::Aes128([0x22; 16]);
        let correct_params = AesParams {
            key: correct_key.clone(),
        };
        let wrong_params = AesParams { key: wrong_key };

        let plaintext = "This message won't decrypt correctly with the wrong key";
        let ct = aes_encrypt_cbc(plaintext, correct_params.clone()).expect("Encryption failed");

        let result = aes_decrypt_cbc(&ct, wrong_params);
        assert!(result.is_err());
    }
}
