local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local msg =
  kumo.make_message('sender@example.com', 'recip@example.com', 'woot')
utils.assert_eq(tostring(msg:recipient()), 'recip@example.com')

msg:set_recipient { 'a@example.com' }
utils.assert_eq(tostring(msg:recipient()), 'a@example.com')

msg:set_recipient { 'a@example.com', 'b@example.com' }
utils.assert_eq(
  utils.dumps(msg:recipient()),
  '{\n  [1] = "a@example.com",\n  [2] = "b@example.com"\n}'
)
