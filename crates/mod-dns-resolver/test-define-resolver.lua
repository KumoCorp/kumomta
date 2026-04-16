local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local st, err = pcall(kumo.dns.lookup_ptr, '10.0.0.1', 'woot')
assert(not st, err)
utils.assert_matches(tostring(err), 'resolver woot is not defined')

kumo.dns.define_resolver('woot', {
  Test = {
    zones = {
      [[
$ORIGIN 0.0.10.in-addr.arpa.
1 30 IN PTR hello.
  ]],
    },
  },
})

local r = kumo.dns.lookup_ptr('10.0.0.1', 'woot')
utils.assert_eq(r, { 'hello.' })

local r = kumo.dns.lookup_ptr '10.0.0.1'
utils.assert_ne(r, { 'hello.' })
