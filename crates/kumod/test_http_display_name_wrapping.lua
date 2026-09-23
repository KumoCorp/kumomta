local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ok, msgs = pcall(kumo.api.inject.build_v1, {
  envelope_sender = 'no-reply@example.com',
  recipients = {
    { email = 'user@example.com', name = 'John Smith' },
  },
  content = {
    text_body = 'Hello',
    -- This produces a From header long enough to require folding. The fold
    -- is inserted at the display-name/<addr> boundary, leaving the display
    -- name itself on one line.
    from = {
      email = 'example@example.com',
      name = 'Redacted Redacted Redacted Redacted Redacted | Redacted Redacted Redacteddd',
    },
  },
})

assert(ok)
local msg = msgs[1]
-- Rebuild re-parses and re-emits the header. Check the display name survives
-- the round trip intact.
local rebuilt = msg:parse_mime():rebuild()
msg:set_data(tostring(rebuilt))
local from = msg:from_header()
utils.assert_eq(from.domain, 'example.com')
utils.assert_eq(
  from.name,
  'Redacted Redacted Redacted Redacted Redacted | Redacted Redacted Redacteddd'
)
