local kumo = require 'kumo'

-- Create temp directory and key file
local temp_dir = os.tmpname() .. "_modcrypto"
os.execute("mkdir -p " .. temp_dir)

-- Generate a 256-bit (32 bytes) AES key and save to temp file
local hex_key_256 = '2b7e151628aed2a6abf7158809cf4f3c2b7e151628aed2a6abf7158809cf4f3c'  -- 256-bit key (64 hex chars = 32 bytes)
local key_file = temp_dir .. "/aes_key_256.bin"

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

-- Create key file
local binary_key = hex_to_binary(hex_key_256)

-- Debug: Check key lengths before creating file
print("Hex key length:", #hex_key_256, "characters")
print("Binary key length:", #binary_key, "bytes")
print("Expected binary length for 256-bit:", #hex_key_256 / 2, "bytes")

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

-- Test messages
local hex_key_128 = '1234567890abcdef'  -- 16 hex chars = 16 ASCII bytes = AES-128 compatible
local original_message = 'Hello, World!123' 
local second_message = 'second message here!!!'

-- Debug: Check 128-bit key length
print("128-bit hex key length:", #hex_key_128, "characters")
print("As ASCII bytes this will be:", #hex_key_128, "bytes (AES-128 compatible)")

-- Test 1: File-based key (CBC)
print("Testing file-based key...")
local f = kumo.crypto.aes_encrypt_block {
  key = key_file,
  value = original_message,
  algorithm = 'Cbc',
}

local b = kumo.crypto.aes_decrypt_block {
  key = key_file,
  value = f,
  algorithm = 'Cbc',
}

if b.bytes ~= original_message then
    print(string.format("FAILED: File-based CBC fail : %s != %s", b.bytes, original_message))
    cleanup()
    os.exit(1)
end
print(string.format("[OK] -- CBC encryption test OK: with msg: \"%s\"", b.bytes))

-- Test 2: Hex key (ECB) - use hex string directly as KeySource::Data expects
print("Testing hex-based key...")
print("Using hex key directly:", hex_key_128, "(length:", #hex_key_128, "chars)")

local f2 = kumo.crypto.aes_encrypt_block {
  key = {
    key_data = hex_key_128,  -- Use hex string directly, not binary!
  },
  value = second_message,
  algorithm = 'Ecb',
}

local dr = kumo.crypto.aes_decrypt_block {
  key = {
    key_data = hex_key_128,  -- Use hex string directly, not binary!
  },
  value = f2,
  algorithm = 'Ecb',
}

if dr.bytes ~= second_message then
    print("FAILED: Hex-based ECB round-trip failed!")
    print("Expected:", second_message)
    print("Got:", dr.bytes)
    cleanup()
    os.exit(1)
end

print(string.format("Hex: %s", dr.hex))
print(string.format("Base64: %s", dr.base64))
print(string.format("Base64 (no pad): %s", dr.base64_nopad))
print(string.format("Bytes length: %s", #dr.bytes))
print(string.format("Final result: %s", dr.bytes))
cleanup()
print("-- mod_crypto--  All tests lua passed!")