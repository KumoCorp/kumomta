local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

-- HeaderAddressList: single simple address without display name
local msg =
  new_msg 'X-Campaign: 1234\r\nX-Campaign: 9876\r\nX-Header: test\r\nFrom: user@example.com\r\nTo: someone@example.com\r\n\r\nBody'

msg:import_x_headers { 'X-Campaign' }

utils.assert_eq(msg:get_meta 'x_campaign', '9876')

msg:import_x_headers({ 'X-Campaign' }, true)
utils.assert_eq(msg:get_meta 'x_campaign', '1234')

msg:import_x_headers(nil, true)
utils.assert_eq(msg:get_meta 'x_header', 'test')
