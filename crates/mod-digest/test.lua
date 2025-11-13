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

local key_bytes = {
  key_data = 'your key',
}

utils.assert_eq(
  kumo.digest.hmac_sha1(key_bytes, 'your message').hex,
  '317d0dfd868a5c06c9444ac1328aa3e2bfd29fb2'
)

utils.assert_eq(
  kumo.digest.hmac_sha224(key_bytes, 'your message').hex,
  '991ff8089bd2a781d10c0568cbf2717794ac7b7fbcc9db049f19fc61'
)

utils.assert_eq(
  kumo.digest.hmac_sha256(key_bytes, 'your message').hex,
  '87fc1cec5c02f0991ae80f50e98eb2eb5213d07fc40417682a74448ac1deb07c'
)

utils.assert_eq(
  kumo.digest.hmac_sha384(key_bytes, 'your message').hex,
  'cd274957b95ce192d41dd52f83fd2eb9277aa2fa210ec798ee16e978801a89b7e7b956af3976d1a50a60ece87c7b2a66'
)

utils.assert_eq(
  kumo.digest.hmac_sha512(key_bytes, 'your message').hex,
  '2f5ddcdbd062a5392f07b0cd0262bf52c21bfb3db513296240cca8d5accc09d18d96be0a94995be4494c032f1eda946ad549fb61ccbe985d160f0b2f9588d34b'
)
