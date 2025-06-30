use config::get_or_create_sub_module;
use mlua::{Error as LuaError}; 
use mlua::Lua;
use data_loader::KeySource;


use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce, Key,
};
use std::fs::File;
use std::io::{Read};
// Import the base64 crate Engine trait anonymously so we can
// call its methods without adding to the namespace.
use base64::{engine::general_purpose::STANDARD, Engine as _};


// TODO 
// use https://github.com/KumoCorp/kumomta/blob/main/crates/data-loader/src/lib.rs#L13 KeySource
// add unit-tests
pub struct AesParam {
pub aes_key_source: Option<KeySource>,
}




/// aes_encrypt
/// 1) TODO: use the Keystore input instead of enc_key
/// 2) TODO: depending on the key bits use the correct Chiper creation
pub fn aes_encrypt(value: String, enc_key: String, key_length_bits: usize) -> Result< String, aes_gcm::Error> {
    // Determine expected key size in bytes
    let key_size_bytes = match key_length_bits {
        128 => 16,
        192 => 24,
        256 => 32,
        _ => panic!("Unsupported key size: {key_length_bits} bits"),
    };

    // Read the key from the file
    let mut file = File::open(enc_key).map_err(|_| aes_gcm::Error)?;
    let mut key_bytes = vec![0u8; key_size_bytes];
    file.read_exact(&mut key_bytes).map_err(|_| aes_gcm::Error)?;

    // 
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // Generate a random 96-bit nonce
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 12 bytes

    // Encrypt the value
    let ciphertext = cipher.encrypt(&nonce, value.as_bytes())?;
    
  // Prepend nonce to ciphertext (optional, but required for decryption later)
    let mut nonce_and_ciphertext = nonce.to_vec();
    nonce_and_ciphertext.extend_from_slice(&ciphertext);


    // Encode the combined result to base64
    let encoded = STANDARD.encode(nonce_and_ciphertext);

    println!("Encryption OK!");
    Ok(encoded)
}

pub fn aes_decrypt(encoded: String, enc_key: String, key_length_bits: usize) -> Result<String, aes_gcm::Error> {
    // Determine expected key size in bytes
    let key_size_bytes = match key_length_bits {
        128 => 16,
        192 => 24,
        256 => 32,
        _ => panic!("Unsupported key size: {key_length_bits} bits"),
    };

    // Read key from file
    let mut file = File::open(enc_key).map_err(|_| aes_gcm::Error)?;
    let mut key_bytes = vec![0u8; key_size_bytes];
    file.read_exact(&mut key_bytes).map_err(|_| aes_gcm::Error)?;

    // Create cipher
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // Decode base64 input
    let nonce_and_ciphertext = STANDARD.decode(encoded).map_err(|_| aes_gcm::Error)?;

    // Split nonce and ciphertext
    if nonce_and_ciphertext.len() < 12 {
        return Err(aes_gcm::Error); // Too short to contain valid nonce + data
    }
    let (nonce_bytes, ciphertext) = nonce_and_ciphertext.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Decrypt
    let plaintext_bytes = cipher.decrypt(nonce, ciphertext.as_ref())?;
    let plaintext = String::from_utf8(plaintext_bytes).map_err(|_| aes_gcm::Error)?;

    println!("Decryption OK!");
    Ok(plaintext)
}




// Todo remove enc_key argument.
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let crypto = get_or_create_sub_module(lua, "crypto")?;
       crypto.set(
            "aes_encrypt",
            lua.create_function(|_, (value, enc_key, key_length_bits): (String, String, usize)| {
                  let result = aes_encrypt(value, enc_key, key_length_bits)
                     .map_err(|e| LuaError::external(e.to_string()))?;
             Ok(result)
             })?,
    )?;
       crypto.set(
        "aes_decrypt",
        lua.create_function(|_, (value, enc_key ,  key_length_bits): (String, String, usize)| {
                let result= aes_decrypt(value, enc_key, key_length_bits)
                .map_err(|e| LuaError::external(e.to_string()))?;
             Ok(result)
        })?,
    )?;

    Ok(())
}