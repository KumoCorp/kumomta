local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

local msg = new_msg 'Subject: hello\r\n\r\nHello'

local long_string = string.rep('A', 1500)
local expect_wrapped = string.rep('A', 900) .. ' ' .. string.rep('A', 600)
msg:prepend_header('X-Long', long_string)
utils.assert_eq(
  tostring(msg:get_first_named_header_value 'X-Long'),
  long_string
)
msg:prepend_header('X-Long-Wrap', long_string, true)
utils.assert_eq(
  tostring(msg:get_first_named_header_value 'X-Long-Wrap'),
  expect_wrapped
)

msg:prepend_header('X-Emoji-Unencoded', '👾')
-- The value is stored as UTF-8
utils.assert_eq(
  tostring(msg:get_first_named_header_value 'X-Emoji-Unencoded'),
  '👾'
)
utils.assert_eq(
  msg:parse_mime().headers:get_first_named('X-Emoji-Unencoded').raw_value,
  '👾'
)

msg:prepend_header('X-Emoji', '👾', true)
utils.assert_eq(tostring(msg:get_first_named_header_value 'X-Emoji'), '👾')
utils.assert_eq(
  msg:parse_mime().headers:get_first_named('X-Emoji').raw_value,
  '=?UTF-8?q?=F0=9F=91=BE?='
)

-- Soft wrap lines with spaces
local long_string = string.rep(' hello there', 10)
msg:prepend_header('X-Long-Space', long_string)
utils.assert_eq(
  tostring(msg:get_first_named_header_value 'X-Long-Space'),
  'hello there hello there hello there hello there hello there hello there '
    .. 'hello there hello there hello there hello there'
)

msg:append_header('X-Long-Space-Wrap', long_string, true)
utils.assert_eq(
  tostring(msg:get_first_named_header_value 'X-Long-Space-Wrap'),
  'hello there hello there hello there hello there hello there hello there '
    .. 'hello there hello there hello there hello there'
)
