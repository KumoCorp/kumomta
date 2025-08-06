local kumo = require 'kumo'
local crc32 = kumo.digest.crc32 'something'

assert(
  crc32.hex == '09da31fb',
  'crc32 of "something" is 09da31fb, got ' .. crc32.hex
)
