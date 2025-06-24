use config::get_or_create_sub_module;
use mlua::Lua;
use std::fmt;

// TODO 
// use https://github.com/KumoCorp/kumomta/blob/main/crates/data-loader/src/lib.rs#L13 KeySource
// add unit-tests


#[derive(Debug)]
pub enum AesError {
    InvalidKeyLength,
}


impl std::error::Error for AesError {}
impl fmt::Display for AesError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AesError::InvalidKeyLength => write!(f, "Invalid AES key length. Must be 16, 24, or 32 bytes (128, 192, or 256 bits)."),
        }
    }
}

/// (WORK IN PROGRESS)
pub fn aes_encrypt(value: &[u8], enc_key: &[u8], key_length_bits: usize) -> Result<Vec<u8>, AesError> {
    let expected_key_bytes = key_length_bits / 8;
    if enc_key.len() != expected_key_bytes {
        // Return an error for invalid key length, matching the Result<T, E> signature
        return Err(AesError::InvalidKeyLength);
    }

    let encrypted_data = b"Hello, World!".to_vec();

    // Return the successful result, matching the Result<T, E> signature
    Ok(encrypted_data)
}

pub fn aes_decrypt(value: &[u8], enc_key: &[u8], key_length_bits: usize) -> Result<Vec<u8>, AesError> {
    // Basic key length validation
    let expected_key_bytes = key_length_bits / 8;
    if enc_key.len() != expected_key_bytes {
        return Err(AesError::InvalidKeyLength);
    }

    // Simulate successful decryption
    let decrypted_data = b"Decrypted World!".to_vec();
    Ok(decrypted_data)
}



pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;
       crypto.set(
        "aes_encrypt",
        lua.create_function(|_, (value, enc_key, key_length_bits): (String, String, usize)| {
        aes_encrypt( value.as_bytes(), enc_key.as_bytes(), key_length_bits);
        Ok(())
            })?,
        )?;
       crypto.set(
        // aes_encrypt(value, enc_key, key_length)
        "aes_decrypt",
        // value
                lua.create_function(|_, (value, enc_key ,  key_length_bits): (String, String, usize)| {
        aes_decrypt(value.as_bytes(), enc_key.as_bytes(), key_length_bits);
            Ok(())
        })?,
    )?;

    Ok(())
}
