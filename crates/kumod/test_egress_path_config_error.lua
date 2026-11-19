local kumo = require 'kumo'

local ok, err = pcall(kumo.make_egress_path, {
  smtp_port = 'invalid',
  smtp_auth_plain_password = { key_data = 'test-relay-password' },
})

assert(not ok)
local detail = tostring(err)
assert(detail:find('smtp_port', 1, true), detail)
assert(detail:find('expected u16', 1, true), detail)
assert(not detail:find('test-relay-password', 1, true), detail)
