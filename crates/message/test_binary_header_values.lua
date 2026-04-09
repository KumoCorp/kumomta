local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

-- Latin-1 bytes (non-UTF-8) pass through get_first_named_header_value
-- without lossy encoding and without error.
-- 0xA9 is latin-1 copyright, 0xE9 is latin-1 é
local msg =
  new_msg 'Subject: hello \xa9 world \xe9\r\nX-Custom: \xff\xfe\r\n\r\nBody'
utils.assert_eq(
  msg:get_first_named_header_value 'subject',
  'hello \xa9 world \xe9'
)
utils.assert_eq(msg:get_first_named_header_value 'X-Custom', '\xff\xfe')

-- get_all_named_header_values returns binary content from multiple headers
local msg2 =
  new_msg 'X-Bin: \x80\x81\r\nX-Bin: \x90\x91\r\nSubject: test\r\n\r\nBody'
local values = msg2:get_all_named_header_values 'X-Bin'
utils.assert_eq(#values, 2)
utils.assert_eq(values[1], '\x80\x81')
utils.assert_eq(values[2], '\x90\x91')

-- get_all_headers preserves binary in both names and values
-- (header names are always ASCII in practice, but values can be binary)
local msg3 = new_msg 'X-Data: \xde\xad\xbe\xef\r\nSubject: ok\r\n\r\nBody'
local all = msg3:get_all_headers()
-- Find the X-Data header
local found = false
for _, pair in ipairs(all) do
  if pair[1] == 'X-Data' then
    utils.assert_eq(pair[2], '\xde\xad\xbe\xef')
    found = true
  end
end
utils.assert_eq(
  found,
  true,
  'X-Data header should be found in get_all_headers'
)

-- Verify that pure ASCII values also work fine alongside binary
utils.assert_eq(
  msg:get_first_named_header_value 'subject',
  'hello \xa9 world \xe9'
)

-- A header with all 256 byte values (0x01-0xff, skipping 0x00 which
-- terminates C strings, and \r\n which are line terminators)
local bytes = {}
for i = 1, 255 do
  -- Skip CR (13) and LF (10) as they would break the header
  if i ~= 10 and i ~= 13 then
    table.insert(bytes, string.char(i))
  end
end
local all_bytes = table.concat(bytes)
local msg4 = new_msg('X-AllBytes: ' .. all_bytes .. '\r\n\r\nBody')
local result = msg4:get_first_named_header_value 'X-AllBytes'
utils.assert_eq(result, all_bytes)

-- Authentication-Results header with binary (non-UTF-8) content in serv_id
-- and reason fields. Binary in propspec values is tested with a leading
-- non-ASCII byte to avoid the parser's domain-before-value alternation.
local ar_header = 'mx.ex\x80mple.com;'
  .. ' spf=pass reason=good\xffsig'
  .. ' smtp.mailfrom=\xfevalue'
local msg5 =
  new_msg('Authentication-Results: ' .. ar_header .. '\r\n\r\nBody')
local ar_value = msg5:get_first_named_header_value 'Authentication-Results'
utils.assert_eq(ar_value, ar_header)

-- Use the structured authentication_results accessor to verify binary
-- is preserved in individual parsed fields.
local hdr = msg5:parse_mime().headers:get_first_named 'Authentication-Results'
local ar = hdr.authentication_results
utils.assert_eq(ar.serv_id, 'mx.ex\x80mple.com')
utils.assert_eq(#ar.results, 1)
local r = ar.results[1]
utils.assert_eq(r.method, 'spf')
utils.assert_eq(r.result, 'pass')
utils.assert_eq(r.reason, 'good\xffsig')
utils.assert_eq(r.props['smtp.mailfrom'], '\xfevalue')
