# aes_decrypt_block

{{since('dev')}}

```lua
kumo.crypto.aes_decrypt_block(ALGORITHM, CIPHERTEXT, KEY)
```

`ALGORITHM` can be one of:

 * `'Cbc'` - Cipher block chaining (CBC).
 * `'Ecb'` - Electronic Code Book (ECB).

`CIPHERTEXT` is the (likely binary) string holding the encrypted payload that
you wish to decrypt.

`KEY` describes the decryption key. It must be [keysource](../keysource.md)
object that references the source of the key.  Supported key sizes are 16 or 32
binary bytes, allowing for AES-128 or AES-256 ciphers.

The return value is the decrypted data.

The [kumo.crypto.aes_encrypt_block](aes_encrypt_block.md) function can be used
to encrypt data suitable for decrypting with `kumo.crypto.aes_decrypt_block`.


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
