local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local cfg =
  kumo.dns.load_resolv_conf 'crates/mod-dns-resolver/test-load-resolv-conf.conf'

utils.assert_eq(cfg.name_servers, { '192.0.2.1:53', '198.51.100.53:53' })
utils.assert_eq(cfg.search, { 'example.com', 'sub.example.com' })
utils.assert_eq(cfg.options.ndots, 3)
utils.assert_eq(cfg.options.timeout, '7s')
utils.assert_eq(cfg.options.attempts, 4)
utils.assert_eq(cfg.options.edns0, true)

-- Round-trip: the loaded table should be accepted by configure_resolver.
kumo.dns.configure_resolver(cfg)
