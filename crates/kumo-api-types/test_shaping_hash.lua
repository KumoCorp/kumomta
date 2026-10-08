local kumo = require 'kumo'

-- A hermetic resolver that answers only for keep.example. An MX lookup for
-- any other domain, including failmx.example below, gets NXDOMAIN from it.
kumo.dns.configure_test_resolver {
  [[
$ORIGIN keep.example.
@ 60 IN TXT "placeholder"
  ]],
}

-- failmx.example below is the domain we expect shaping to drop and warn
-- about. Unlike keep.example, it does not set mx_rollup = false: it keeps the
-- default of true, and shaping resolves its MX.
local path = os.tmpname()
local f = assert(io.open(path, 'w'))
assert(f:write [[
["keep.example"]
mx_rollup = false
connection_limit = 7

["failmx.example"]
connection_limit = 3
]])
f:close()

-- dns_fail only controls whether the NXDOMAIN is recorded as a warning. In both
-- cases shaping drops failmx.example from the merged configuration.
local warned = kumo.shaping.load({ path }, { dns_fail = 'Warn' })
local ignored = kumo.shaping.load({ path }, { dns_fail = 'Ignore' })

os.remove(path)

assert(
  #warned:get_warnings() > 0,
  'expected a dns_fail warning when dns_fail = Warn'
)
assert(
  #ignored:get_warnings() == 0,
  'expected no warnings when dns_fail = Ignore'
)
-- warned and ignored differ only in their warnings, which the hash must not
-- cover: the hashes should match.
assert(
  warned:hash() == ignored:hash(),
  string.format(
    'hash must ignore diagnostics: warned=%s ignored=%s',
    warned:hash(),
    ignored:hash()
  )
)
