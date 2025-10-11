local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- While not strictly required, we mock out the real DNS with a
-- test resolver named rbl here so that this test isn't dependent
-- upon the network.  The expectations match what is returned from
-- the real zone for the queries used in this file
kumo.dns.define_resolver('rbl', {
  Test = {
    zones = {
      [[
$ORIGIN bl.spamcop.net.
2.0.0.127   600    A 127.0.0.2
2.0.0.127   600    TXT "Blocked - see https://www.spamcop.net/bl.shtml?127.0.0.2"
]],
    },
  },
})

local ip, reason =
  kumo.dns.rbl_lookup('127.0.0.2:25', 'bl.spamcop.net', 'rbl')
print(ip, reason)
utils.assert_eq(ip, '127.0.0.2')
utils.assert_eq(
  reason,
  'Blocked - see https://www.spamcop.net/bl.shtml?127.0.0.2'
)

local ip, reason = kumo.dns.rbl_lookup('127.0.0.3', 'bl.spamcop.net', 'rbl')
print(ip, reason)
utils.assert_ne(ip, '127.0.0.2')
utils.assert_eq(reason, nil)
