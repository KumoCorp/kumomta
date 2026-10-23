local kumo = require 'kumo'

-- Exercises the DNSSEC trust anchor options on the resolver backends. This runs
-- offline: it checks config-time behavior (managed-file seeding), not live
-- DNSSEC resolution.

-- A managed (RFC 5011) anchor file on the unbound backend is seeded with the
-- bundled root anchors when it does not yet exist.
local managed = os.tmpname()
kumo.dns.configure_resolver {
  Unbound = {
    options = {
      trust_anchor_file = { managed = managed },
    },
  },
}

local f =
  assert(io.open(managed, 'r'), 'managed anchor file should be seeded')
local seeded = f:read 'a'
f:close()
assert(seeded:find 'IN DS', 'seeded anchor file should contain DS records')

-- Seeding must not clobber an existing (non-empty) anchor file.
local existing = os.tmpname()
local known = '. IN DS 12345 8 2 abcdef\n'
local w = assert(io.open(existing, 'w'))
w:write(known)
w:close()

kumo.dns.configure_resolver {
  Unbound = {
    options = {
      trust_anchor_file = { managed = existing },
    },
  },
}

local f2 = assert(io.open(existing, 'r'))
local after = f2:read 'a'
f2:close()
assert(
  after == known,
  'an existing managed anchor file must not be overwritten'
)

-- The static (string) form is accepted; reuse the seeded file, which holds
-- valid DS records.
kumo.dns.configure_resolver {
  Unbound = {
    options = {
      validate = true,
      trust_anchor_file = managed,
    },
  },
}

os.remove(managed)
os.remove(existing)

-- Opt-in end-to-end validation against the live DNS. This actually resolves
-- DNSSEC-signed and unsigned names through a validating unbound resolver that
-- uses a managed anchor file, proving the whole path works. It depends on the
-- network so it is gated behind LIVE_DNS_TESTS; run with
-- `LIVE_DNS_TESTS=1 make test-lua`.
if os.getenv 'LIVE_DNS_TESTS' then
  local anchor = os.tmpname()
  -- No name_servers: unbound recurses from the root, so it actually validates
  -- the full chain against the seeded trust anchors.
  kumo.dns.configure_resolver {
    Unbound = {
      options = {
        trust_anchor_file = { managed = anchor },
      },
    },
  }

  -- A DNSSEC-signed domain validates as secure.
  local signed = kumo.dns.lookup_mx 'do.havedane.net'
  assert(signed.is_secure, 'do.havedane.net should validate as secure')

  -- An unsigned domain still resolves, but is not secure.
  local unsigned = kumo.dns.lookup_mx 'google.com'
  assert(not unsigned.is_secure, 'google.com is not DNSSEC signed')

  os.remove(anchor)
end
