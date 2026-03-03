local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ok, msgs = pcall(kumo.api.inject.build_v1, {
  envelope_sender = 'no-reply@example.com',
  recipients = {
    { email = 'user@example.com', name = 'John Smith' },
  },
  content = {
    text_body = 'Hello',
    -- This produces a From header that requires wrapping.
    -- The wrapped header would simply use \n\t to wrap,
    -- which is appropriate for this case.
    from = {
      email = 'example@example.com',
      name = 'Redacted Redacted Redacted Redacted Redacted | Redacted Redacted Redacteddd',
    },
  },
})

assert(ok)
local msg = msgs[1]
-- The rebuild operation incorrectly rebuilt the \n\t wrapping
-- to use a backslash quote and messed things up.  This test
-- is verifying that we can successfully parse the rebuilt
-- version of the header
local rebuilt = msg:parse_mime():rebuild()
msg:set_data(tostring(rebuilt))
local from = msg:from_header()
utils.assert_eq(from.domain, 'example.com')
utils.assert_eq(
  from.name,
  'Redacted Redacted Redacted Redacted Redacted | Redacted Redacted\r\n\tRedacteddd'
)
