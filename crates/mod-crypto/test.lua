local kumo = require 'kumo'

-- helpers 
-- Create temp directory and key file
local temp_dir = os.tmpname() .. "_modcrypto"
os.execute("mkdir -p " .. temp_dir)
-- Convert hex to binary and write to file
local function hex_to_binary(hex)
    return (hex:gsub('..', function (cc)
        return string.char(tonumber(cc, 16))
    end))
end
-- Cleanup function
local function cleanup()
    os.execute("rm -rf " .. temp_dir)
    print("Cleaned up temp directory: " .. temp_dir)
end

-- Generate a 256-bit (32 bytes) AES key and save to temp file
local hex_key_256 = '2b7e151628aed2a6abf7158809cf4f3c2b7e151628aed2a6abf7158809cf4f3c'  -- 256-bit key (64 hex chars = 32 bytes)
local key_file = temp_dir .. "/aes_key_256.bin"

-- Create key file
local binary_key = hex_to_binary(hex_key_256)
-- Verify we have exactly 32 bytes for AES-256
if #binary_key ~= 32 then
    cleanup()
    error("Generated key is " .. #binary_key .. " bytes, expected 32 bytes for AES-256")
end

local file = io.open(key_file, "wb")
if not file then
    cleanup()
    error("Failed to create key file: " .. key_file)
end
file:write(binary_key)
file:close()
print("Created temp key file: " .. key_file)

--------------
-- Test 1: File-based key (CBC)

-- Test messages
local hex_key_128 = '1234567890abcdef'  -- 16 hex chars = 16 ASCII bytes = AES-128 compatible
local original_message = 'Hello, World!123' 
local second_message = 'second message here!!!'

print("Testing file-based key...")
local f = kumo.crypto.aes_encrypt_block('Cbc', original_message, {
  key = key_file,
})

local b = kumo.crypto.aes_decrypt_block('Cbc', f, {
  key = key_file,
})

if b.bytes ~= original_message then
    error("FAILED: File-based CBC fail : " .. b.bytes .. " != " .. original_message)
end
print(string.format("[OK] -- CBC encryption test OK: with msg: \"%s\"", b.bytes))

-- Test 2: Hex key (ECB) 
print("Testing hex-based key...")
print("Using hex key directly:", hex_key_128, "(length:", #hex_key_128, "chars)")

local f2 = kumo.crypto.aes_encrypt_block('Ecb', second_message, {
  key = {
    key_data = hex_key_128,  -- Use hex string directly, not binary!
  },
})

local dr = kumo.crypto.aes_decrypt_block('Ecb', f2, {
  key = {
    key_data = hex_key_128,  -- Use hex string directly, not binary!
  },
})

if dr.bytes ~= second_message then
    print("FAILED: Hex-based ECB round-trip failed!")
    print("Expected:", second_message)
    print("Got:", dr.bytes)
    os.exit(1)
end

print(string.format("Hex: %s", dr.hex))
print(string.format("Base64: %s", dr.base64))
print(string.format("Base64 (no pad): %s", dr.base64_nopad))
print(string.format("Bytes length: %s", #dr.bytes))
print(string.format("Final result: %s", dr.bytes))

-- Test 3: Binary non-UTF-8 data
print("Testing with binary non-UTF-8 data...")
local binary_data = string.char(0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0xFD, 0xFC, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0)
print("Binary data length:", #binary_data, "bytes")
print("Binary data hex:", kumo.encode.hex_encode(binary_data))

local f3 = kumo.crypto.aes_encrypt_block('Ecb', binary_data, {
  key = {
    key_data = hex_key_128,
  },
})

local dr3 = kumo.crypto.aes_decrypt_block('Ecb', f3, {
  key = {
    key_data = hex_key_128,
  },
})

if dr3.bytes ~= binary_data then
    print("FAILED: Binary data ECB round-trip failed!")
    print("Expected length:", #binary_data)
    print("Got length:", #dr3.bytes)
    print("Expected hex:", kumo.encode.hex_encode(binary_data))
    print("Got hex:", dr3.hex)
    os.exit(1)
end
print("UCCESS: Binary non-UTF-8 data test passed!")

cleanup()
print("-- mod_crypto--  All tests lua passed!")