local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

-- 0xA9 is latin-1 for the copyright symbol
local msg = new_msg 'Subject: \xa9\r\n\r\nGBP'

-- We return the latin-1 bytes as-is
utils.assert_eq(msg:get_first_named_header_value 'subject', '\xa9')

msg:check_fix_conformance('', 'NEEDS_TRANSFER_ENCODING', {
  detect_encoding = true,
  include_encodings = {
    'iso-8859-1',
  },
  exclude_encodings = {},
})

-- The result is now the utf-8 form of the latin-1 input,
-- and the copyright symbol comes through
utils.assert_eq(msg:get_first_named_header_value 'subject', '©')
