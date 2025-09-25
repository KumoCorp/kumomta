local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local msg = kumo.make_message(
  'sender@example.com',
  'recip@example.com',
  'Subject: hello\r\n\r\nHi'
)
kumo.apply_supplemental_trace_header(msg)
local trace = msg:get_first_named_header_value 'X-KumoRef'

utils.assert_eq(
  trace,
  'eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20ifQ=='
)

msg:set_meta('woot', 'woot')
kumo.apply_supplemental_trace_header(msg, { include_meta_names = { 'woot' } })
local trace = msg:get_first_named_header_value 'X-KumoRef'

utils.assert_eq(
  trace,
  'eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoicmVjaXBAZXhhbXBsZS5jb20iLCJ3b290Ijoid29vdCJ9'
)
