local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

kumo.dns.define_resolver('woot', {
  Test = {
    zones = {
      [[
$ORIGIN example.com.
mx1 30 IN A 10.0.0.1
mx1 30 IN AAAA ::1
mx4 30 IN A 10.0.0.2
mx6 30 IN AAAA ::1
  ]],
    },
  },
})

utils.assert_eq(
  kumo.dns.lookup_addr('mx1.example.com', 'woot', 'Ipv4AndIpv6'),
  { '10.0.0.1', '::1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx1.example.com', 'woot', 'Ipv4Only'),
  { '10.0.0.1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx1.example.com', 'woot', 'Ipv4ThenIpv6'),
  { '10.0.0.1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx4.example.com', 'woot', 'Ipv4ThenIpv6'),
  { '10.0.0.2' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx6.example.com', 'woot', 'Ipv4ThenIpv6'),
  { '::1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx1.example.com', 'woot', 'Ipv6Only'),
  { '::1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx6.example.com', 'woot', 'Ipv4Only'),
  {}
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx6.example.com', 'woot', 'Ipv6Only'),
  { '::1' }
)

utils.assert_eq(
  kumo.dns.lookup_addr('mx4.example.com', 'woot', 'Ipv6Only'),
  {}
)
