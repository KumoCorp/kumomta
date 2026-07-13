# rbl_lookup

```lua
local answer, reason = kumo.dns.rbl_lookup(IP, BASE_DOMAIN, OPT_RESOLVER_NAME)
```

{{since('2025.12.02-67ee9e96')}}

This is a convenience function that enables looking up an IP address in a
[DNSBL](https://en.wikipedia.org/wiki/Domain_Name_System_blocklist).

The `IP` parameter is either an IPv4 or IPv6 address string, and for the sake
of convenience, allows a socket address with optional port number.

The `BASE_DOMAIN` parameter is the base domain to consult.  The value used here
depends on which RBL is being queried and whether you're using a service in
public DNS or are running a local resolver that is hosting RBL zones.

The `OPT_RESOLVER_NAME` is an optional string parameter that specifies the name
of a alternate resolver defined via [define_resolver](define_resolver.md).  You
can omit this parameter and the default resolver will be used.

The return value is a tuple consisting of the IP address that the lookup
resolves to, if any, and the corresponding TXT record if there was an IP
address.

Generally speaking, the returned IP address will be `127.0.0.2` to indicate
that an IP is blocked, but it may also be some other non-`127.0.0.1` loopback
address to indicate some other listing status depending on the RBL that you are
querying.

If the IP is not present on the RBL, both values of the tuple will be `nil`.

## Querying SpamCop

This example shows how to query
[SpamCop](https://www.spamcop.net/fom-serve/cache/291.html) to see if an IP
address is listed.  This example is using the test IP `127.0.0.2` which is
always listed, in order to demonstrate the expected results when an IP
is blocked:

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ip, reason = kumo.dns.rbl_lookup('127.0.0.2', 'bl.spamcop.net')
print(ip, reason)
utils.assert_eq(ip, '127.0.0.2')
utils.assert_eq(
  reason,
  'Blocked - see https://www.spamcop.net/bl.shtml?127.0.0.2'
)
```

A more real world example might look something like:

```lua
local ip, reason =
  kumo.dns.rbl_lookup(msg:get_meta 'received_from', 'bl.spamcop.net')
if ip then
  kumo.reject(550, string.format('5.7.1 %s', reason))
end
```
