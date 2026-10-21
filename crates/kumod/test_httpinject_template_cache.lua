local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local request = {
  envelope_sender = 'noreply@example.com',
  content = 'Subject: hello {{ name }}\r\n\r\nHi {{ name }}, you are on {{ plan }}\r\n',
  substitutions = { plan = 'gold' },
  recipients = {
    { email = 'user@example.com', name = 'James Smythe' },
  },
}

local function build()
  local msgs = kumo.api.inject.build_v1(request)
  utils.assert_eq(#msgs, 1)
  return msgs[1]:get_data()
end

-- The cache must not change the generated message
kumo.set_httpinject_template_cache_enabled(false)
local uncached = build()
kumo.set_httpinject_template_cache_enabled(true)
local first_cached = build()
local second_cached = build()

utils.assert_eq(first_cached, uncached)
utils.assert_eq(second_cached, uncached)
utils.assert_matches(uncached, 'Hi James Smythe, you are on gold')
