local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function make_msg()
  return kumo.make_message(
    'sender@example.com',
    'recip@example.com',
    'Subject: hello\r\n\r\nHi'
  )
end

-- A recipient-only trace header is short enough to sit on a single line.
local recip_only = make_msg()
kumo.apply_supplemental_trace_header(recip_only)
utils.assert_eq(
  recip_only:get_data(),
  'X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ==\r\nSubject: hello\r\n\r\nHi'
)

-- A short included meta value still exceeds the narrow fold width once
-- base64-encoded, so it folds onto a second continuation line.
local short_meta = make_msg()
short_meta:set_meta('woot', 'woot')
kumo.apply_supplemental_trace_header(
  short_meta,
  { include_meta_names = { 'woot' } }
)
utils.assert_eq(
  short_meta:get_data(),
  'X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20iLCJ3b290Ijoid29\r\n\tvdCJ9\r\nSubject: hello\r\n\r\nHi'
)

-- A large included meta value produces a long base64 payload that is folded
-- to a narrow, readable width across continuation lines. The fold uses a CRLF
-- followed by a TAB and is not part of the base64 payload.
local long_meta = make_msg()
long_meta:set_meta('big', string.rep('B', 800))
kumo.apply_supplemental_trace_header(
  long_meta,
  { include_meta_names = { 'big' } }
)
utils.assert_eq(
  long_meta:get_data(),
  'X-KumoRef: eyJfQF8iOiJcXF8vIiwiYmlnIjoiQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJ\r\n\tCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk\r\n\tJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQ\r\n\tkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJC\r\n\tQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJ\r\n\tCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk\r\n\tJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQ\r\n\tkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJC\r\n\tQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJ\r\n\tCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk\r\n\tJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQ\r\n\tkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJC\r\n\tQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJ\r\n\tCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk\r\n\tJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkIiLCJyZWNpcGllbnQiOiJyZWNpcEBle\r\n\tGFtcGxlLmNvbSJ9\r\nSubject: hello\r\n\r\nHi'
)

-- A configured header_name replaces the default X-KumoRef.
local custom_name = make_msg()
kumo.apply_supplemental_trace_header(custom_name, { header_name = 'X-Trace' })
utils.assert_eq(
  custom_name:get_data(),
  'X-Trace: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ==\r\nSubject: hello\r\n\r\nHi'
)
