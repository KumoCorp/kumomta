local kumo = require 'kumo'

-- 128 key
local hex_key = '2b7e151628aed2a6abf7158809cf4f3c'  -- 256 key
local original_message = 'Hello, World!123' 
local second_message = 'second message here!!!'


-- showcase encryption/decryption with aes_cbc -- 01 file
local f = kumo.crypto.aes_encrypt_block {
  key = '/tmp/kumo/aes_key_256.bin',
  value = original_message,
  algorithm = 'Cbc',
}
-- 'f' is now a raw binary string representing the encrypted data.
-- 'b' will be a DecryptResult object.
local b = kumo.crypto.aes_decrypt_block {
  key = '/tmp/kumo/aes_key_256.bin',
  value = f,
  algorithm = 'Cbc',
}
if b.bytes ~=  original_message then
    print(string.format("FAILED: File-based CBC fail : %s != %s", b.bytes, original_message))
    os.exit(1)
end
print(string.format("[OK] -- Cbc encryption lua-test OK: with msg: \"%s\"", b.bytes))

-- showcase encryption/decryption with aes_cbc -- 02 key as hex
local f2 = kumo.crypto.aes_encrypt_block {
  key = {
    key_data = hex_key,
  },
  value = second_message,
  algorithm = 'Ecb',
}
local dr = kumo.crypto.aes_decrypt_block {
  key = {
    key_data = hex_key,
  },
  value = f2,
  algorithm = 'Ecb',
}

if dr.bytes ~= second_message then
    print("FAILED: Hex-based ECB round-trip failed!")
    os.exit(1)  -- Exit con codice di errore 1
end


-- valid decrypt result later TODO: move to unit test
print(string.format("Hex: %s", dr.hex))
print(string.format("Base64: %s", dr.base64))
print(string.format("Base64 (no pad): %s", dr.base64_nopad))
print(string.format("Bytes length: %s", #dr.bytes))
print(string.format("Final result: %s", dr.bytes))
