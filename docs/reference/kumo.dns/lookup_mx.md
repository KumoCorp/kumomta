# lookup_mx

```lua
kumo.dns.lookup_mx(DOMAIN, OPT_RESOLVER_NAME)
```

Resolve the MX information for the requested `DOMAIN`.

Raises an error if the domain doesn't exist.

Returns a lua table with the structure shown in the example below.

DNS results are cached according to the TTL specified by the DNS record itself.

This example shows the `gmail.com` MX information.  At the time of writing, the
DNS information looks like this:

```console
$ dig +nocomments mx gmail.com.

; <<>> DiG 9.18.12 <<>> +nocomments mx gmail.com.
;; global options: +cmd
;gmail.com.                     IN      MX
gmail.com.              1620    IN      MX      30 alt3.gmail-smtp-in.l.google.com.
gmail.com.              1620    IN      MX      40 alt4.gmail-smtp-in.l.google.com.
gmail.com.              1620    IN      MX      5 gmail-smtp-in.l.google.com.
gmail.com.              1620    IN      MX      10 alt1.gmail-smtp-in.l.google.com.
gmail.com.              1620    IN      MX      20 alt2.gmail-smtp-in.l.google.com.
;; Query time: 0 msec
;; SERVER: 127.0.0.53#53(127.0.0.53) (UDP)
;; WHEN: Wed Mar 15 09:24:03 MST 2023
;; MSG SIZE  rcvd: 161
```

```lua
-- Query the gmail mx
local gmail_mx = kumo.dns.lookup_mx 'gmail.com'

-- This is what we expect it to look like
local example = {
  by_pref = {
    -- Each preference level has a sorted list of hosts
    -- at that level
    [5] = {
      'gmail-smtp-in.l.google.com.',
    },
    [10] = {
      'alt1.gmail-smtp-in.l.google.com.',
    },
    [20] = {
      'alt2.gmail-smtp-in.l.google.com.',
    },
    [30] = {
      'alt3.gmail-smtp-in.l.google.com.',
    },
    [40] = {
      'alt4.gmail-smtp-in.l.google.com.',
    },
  },

  -- The site name represents the hostname/port set, independent of preferences
  site_name = '(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com',

  -- The FQDN that was resolved
  domain_name = 'gmail.com.',

  -- The flattened set of hosts in preference order
  hosts = {
    'gmail-smtp-in.l.google.com.',
    'alt1.gmail-smtp-in.l.google.com.',
    'alt2.gmail-smtp-in.l.google.com.',
    'alt3.gmail-smtp-in.l.google.com.',
    'alt4.gmail-smtp-in.l.google.com.',
  },

  -- true if the domain is a literal IPv4 or IPv6 address such as
  -- `[10.0.0.1]` or `[IPv6:::1]`
  is_domain_literal = false,
  -- true if the hosts are mx records
  is_mx = true,

  -- The applicable MTA-STS policy mode: 'None', 'Testing' or 'Enforce'
  -- {{since('2026.09.22-a276d4a8', inline=True)}}
  mta_sts = 'None',
}

assert(gmail_mx == example)
```

## Site names

{{since('dev')}}

`site_name` is a canonical representation of the accepted MX hostname/port set.
It normalizes hostname case and trailing dots and ignores duplicate destinations,
MX preference values and record ordering. MTA-STS host filtering takes place
before the name is computed. The `hosts` and `by_pref` fields retain their
preference information; name generation does not reorder the connection plan.

Common hostname components are factored out without combining labels from
unrelated hosts. For example, `a.x.example`, `b.x.example` and `c.y.example`
produce `((a|b).x|c.y).example`, not `(a|b|c).(x|y).example`. The `|` notation
lists alternatives and `?` marks an optional component or group. Literal label
bytes outside ASCII letters, digits, hyphens, underscores and `*` are percent-encoded
using two hexadecimal digits. For example, a dot within a label becomes `%2E`,
distinguishing it from a label separator. The result is an identifier, not a DNS
name or a regular expression to use for host authorization.

Domains with the same accepted destinations share a site name even when their
preferred or backup MX arrangements differ. A shared ready queue uses the MX
preference plan from the domain that created it; site naming does not enforce
per-message preference selection. A preference-only DNS update does not create
a new site identity or replace an existing queue's plan.

Site names also scope shaping configuration, queue limits and backoff. Obtain
them through `lookup_mx` rather than hard-coding their representation; see
[traffic shaping scopes](../../userguide/trafficshaping/scoping.md).

## Named resolvers

{{since('2026.09.22-a276d4a8')}}

The optional `OPT_RESOLVER_NAME` parameter names an alternate resolver defined
via [define_resolver](define_resolver.md). When omitted, the default resolver is
used.

```lua
local kumo = require 'kumo'

kumo.dns.define_resolver('my_resolver', {
  Test = {
    zones = {
      [[
$ORIGIN example.com.
@       600  IN MX 2 two.example.com.
                MX 1 one.example.com.
]],
    },
  },
})

local list = kumo.dns.lookup_mx('example.com', 'my_resolver')
```
