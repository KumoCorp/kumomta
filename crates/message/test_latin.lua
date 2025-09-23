local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

local msg = new_msg 'Subject: \xa9\r\n\r\nGBP'

utils.assert_eq(msg:get_first_named_header_value 'subject', '�')

msg:check_fix_conformance('', 'NEEDS_TRANSFER_ENCODING', {
  detect_encoding = true,
  include_encodings = {
    'iso-8859-1',
  },
  exclude_encodings = {},
})

utils.assert_eq(msg:get_first_named_header_value 'subject', '©')
