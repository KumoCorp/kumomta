local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ok, msg_or_err = pcall(kumo.api.inject.build_v1, {
  envelope_sender = 'no-reply@example.com',
  recipients = {
    { email = 'user@example.com', name = 'John Smith' },
  },
  content = {
    text_body = 'Hello {{ First Name }},\nThis is the second line',
  },
})

assert(not ok)
msg_or_err = tostring(msg_or_err)
assert(
  utils.starts_with(
    msg_or_err,
    "failed parsing field 'content.text_body' as template: syntax error: unexpected identifier, expected end of variable block (in template '0' line 1: 'Hello {{ First Name }},')\n"
  ),
  msg_or_err
)

local ok, msg_or_err = pcall(kumo.api.inject.build_v1, {
  envelope_sender = 'no-reply@example.com',
  recipients = {
    { email = 'user@example.com', name = 'John Smith' },
  },
  content = {
    text_body = 'Hello {{ Name }},\nThis is the second line',
    headers = {
      ['X-Woot'] = '{{ First Name }}',
    },
  },
})

assert(not ok)
msg_or_err = tostring(msg_or_err)
assert(
  utils.starts_with(
    msg_or_err,
    "failed parsing field headers['X-Woot'] as template: syntax error: unexpected identifier, expected end of variable block (in template '1' line 1: '{{ First Name }}')\n"
  ),
  msg_or_err
)
