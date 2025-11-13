local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local crc32 = kumo.digest.crc32 'something'

utils.assert_eq(
  crc32.hex,
  '09da31fb',
  'crc32 of "something" is 09da31fb, got ' .. crc32.hex
)

utils.assert_eq(
  kumo.digest.sha1('foo').hex,
  '0beec7b5ea3f0fdbc95d0dd47f3c5bc275da8a33'
)

utils.assert_eq(
  kumo.digest.sha3_384('foo').hex,
  '665551928d13b7d84ee02734502b018d896a0fb87eed5adb4c87ba91bbd6489410e11b0fbcc06ed7d0ebad559e5d3bb5'
)
