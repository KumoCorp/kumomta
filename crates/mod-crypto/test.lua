local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- a 256-bit (32 bytes) AES key
local hex_key_256 =
  '2b7e151628aed2a6abf7158809cf4f3c2b7e151628aed2a6abf7158809cf4f3c'
-- Decode to binary
local binary_key = kumo.encode.hex_decode(hex_key_256)

--------------
-- Test 1: File-based key (CBC)

-- Test messages
local key_128 = '1234567890abcdef' -- 16 hex chars = 16 ASCII bytes = AES-128 compatible
local original_message = 'Hello, World!123'
local second_message = 'second message here!!!'

local f = kumo.crypto.aes_encrypt_block('Cbc', original_message, {
  key = {
    key_data = binary_key,
  },
})

local b = kumo.crypto.aes_decrypt_block('Cbc', f, {
  key = {
    key_data = binary_key,
  },
})

utils.assert_eq(b.bytes, original_message)

-- Test 2: Hex key (ECB)

local f2 = kumo.crypto.aes_encrypt_block('Ecb', second_message, {
  key = {
    key_data = key_128,
  },
})

local dr = kumo.crypto.aes_decrypt_block('Ecb', f2, {
  key = {
    key_data = key_128,
  },
})

utils.assert_eq(dr.bytes, second_message)

if false then
  print(string.format('Hex: %s', dr.hex))
  print(string.format('Base64: %s', dr.base64))
  print(string.format('Base64 (no pad): %s', dr.base64_nopad))
  print(string.format('Bytes length: %s', #dr.bytes))
  print(string.format('Final result: %s', dr.bytes))
end

-- Test 3: Binary non-UTF-8 data
local binary_data = string.char(
  0x00,
  0x01,
  0x02,
  0x03,
  0xFF,
  0xFE,
  0xFD,
  0xFC,
  0x80,
  0x90,
  0xA0,
  0xB0,
  0xC0,
  0xD0,
  0xE0,
  0xF0
)

if false then
  print('Binary data length:', #binary_data, 'bytes')
  print('Binary data hex:', kumo.encode.hex_encode(binary_data))
end

local f3 = kumo.crypto.aes_encrypt_block('Ecb', binary_data, {
  key = {
    key_data = key_128,
  },
})

local dr3 = kumo.crypto.aes_decrypt_block('Ecb', f3, {
  key = {
    key_data = key_128,
  },
})

utils.assert_eq(dr3.bytes, binary_data)
