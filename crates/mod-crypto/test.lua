local kumo = require 'kumo'

-- 128 key
local hex_key = '2b7e151628aed2a6abf7158809cf4f3c'

-- showcase encryption/decryption with aes_cbc -- 01 file
local f = kumo.crypto.aes_encrypt_block {
  key = '/tmp/kumo/aes_key_256.bin',
  value = 'some value here world',
  algorithm = 'cbc',
}
-- 'f' is now a raw binary string representing the encrypted data.
-- 'b' will be a DecryptResult object.
local b = kumo.crypto.aes_decrypt_block {
  key = '/tmp/kumo/aes_key_256.bin',
  value = f,
  algorithm = 'cbc',
}
print(b.bytes)

-- showcase encryption/decryption with aes_cbc -- 02 key as hex
local f2 = kumo.crypto.aes_encrypt_block {
  key = {
    key_data = hex_key,
  },
  value = 'second message here!!!',
  algorithm = 'ecb',
}
local dr = kumo.crypto.aes_decrypt_block {
  key = {
    key_data = hex_key,
  },
  value = f2,
  algorithm = 'ecb',
}

print(string.format("Hex: %s", dr.hex))
print(string.format("Base64: %s", dr.base64))
print(string.format("Base64 (no pad): %s", dr.base64_nopad))
print(string.format("Bytes length: %s", #dr.bytes))
print(string.format("Final result: %s", dr.bytes))
