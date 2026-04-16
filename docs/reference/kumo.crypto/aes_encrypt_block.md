# aes_encrypt_block

{{since('2025.12.02-67ee9e96')}}

```lua
kumo.crypto.aes_encrypt_block(ALGORITHM, PLAINTEXT, KEY)
```

`ALGORITHM` can be one of:

 * `'Cbc'` - Cipher block chaining (CBC).
 * `'Ecb'` - Electronic Code Book (ECB).

`PLAINTEXT` is the string (which may be binary) to be encrypted.

`KEY` describes the encryption key. It must be [keysource](../keysource.md)
object that references the source of the key.  Supported key sizes are 16 or 32
binary bytes, allowing for AES-128 or AES-256 ciphers.

The return value is the encrypted data wrapped into a
[BinaryResult](../kumo.digest/index.md) object with the same format as that of
the `kumo.digest` crate.  You will likely want to access the raw bytes via its
`.bytes` field, as shown in the examples below.

The [kumo.crypto.aes_decrypt_block](aes_decrypt_block.md) function can be used
to reverse the encryption.

## Example: encrypting/decrypting with a key stored in a file

```lua
local message = 'secret message'

local encrypted = kumo.crypto.aes_encrypt_block('Cbc', message, {
  key = '/path/to/key.bin',
})

-- NOTE: encrypted is a BinaryResult object. You will likely want
-- to use encrypted.bytes to access its bytes!

local decrypted = kumo.crypto.aes_decrypt_block('Cbc', encrypted.bytes, {
  key = '/path/to/key.bin',
})

assert(decrypted == message)
```

## Example: encrypting with a key stored in a vault

```lua
local message = 'secret message'

local encrypted = kumo.crypto.aes_encrypt_block('Cbc', message, {
  key = {
    vault_mount = 'secret',
    vault_path = 'keys/some-path',
  },
})

-- NOTE: encrypted is a BinaryResult object. You will likely want
-- to use encrypted.bytes to access its bytes!

local decrypted = kumo.crypto.aes_decrypt_block('Cbc', encrypted.bytes, {
  key = {
    vault_mount = 'secret',
    vault_path = 'keys/some-path',
  },
})

assert(decrypted == message)
```
