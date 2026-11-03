-- When DANE is enabled, a domain whose MX selection is not DNSSEC-secure but
-- whose MX host publishes DANE TLSA is given a distinct, `[dane-insecure]`
-- prefixed site_name. That keeps it out of the ready queue of a DNSSEC-secure
-- sibling that resolves to the same host and would authenticate with DANE.
local kumo = require 'kumo'

kumo.dns.set_dane_enabled(true)

-- secure.example and insecure.example share one MX host, mx.shared.example,
-- which publishes a usable TLSA record. plain.example is a separate,
-- unauthenticated domain on an ordinary MX host with only an A record.
kumo.dns.configure_test_resolver {
  zones = {
    {
      secure = true,
      zone = [[
$ORIGIN secure.example.
@ 3600 IN MX 10 mx.shared.example.
]],
    },
    {
      secure = false,
      zone = [[
$ORIGIN insecure.example.
@ 3600 IN MX 10 mx.shared.example.
]],
    },
    {
      secure = false,
      zone = [[
$ORIGIN plain.example.
@ 3600 IN MX 10 mx.plain.example.
mx 3600 IN A 127.0.0.1
]],
    },
    {
      secure = true,
      zone = string.format(
        [[
$ORIGIN shared.example.
mx 3600 IN A 127.0.0.1
_25._tcp.mx 3600 IN TLSA 3 0 1 %s
]],
        string.rep('0', 64)
      ),
    },
  },
}

local secure_mx = kumo.dns.lookup_mx 'secure.example'
assert(
  secure_mx.site_name == 'mx.shared.example',
  'secure selection is untagged, got: ' .. secure_mx.site_name
)

local insecure_mx = kumo.dns.lookup_mx 'insecure.example'
assert(
  insecure_mx.site_name == '[dane-insecure]mx.shared.example',
  'insecure selection onto a DANE host is tagged, got: '
    .. insecure_mx.site_name
)

assert(
  secure_mx.site_name ~= insecure_mx.site_name,
  'the secure and insecure domains must not share a site_name'
)

local plain_mx = kumo.dns.lookup_mx 'plain.example'
assert(
  plain_mx.site_name == 'mx.plain.example',
  'an insecure domain on a non-DANE host is not tagged, got: '
    .. plain_mx.site_name
)

kumo.dns.set_dane_enabled(false)
