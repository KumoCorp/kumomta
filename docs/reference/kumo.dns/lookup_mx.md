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

  -- The site name is deterministically derived from the by_pref information
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
}

assert(gmail_mx == example)
```

{{since('dev')}}

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
