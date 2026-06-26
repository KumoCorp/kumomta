# enable_dane

{{since('2023.11.28-b5252a41')}}

When set to `true` (the default is `false`), then `TLSA` records will be
resolved securely to determine the destination site policy for TLS according
to [DANE](https://datatracker.ietf.org/doc/html/rfc7672).

The table below applies only when `enable_dane = true` *and* the destination is
resolved via DNS `MX` records (it does not apply when an explicit `mx_list` is
in use). A *secure chain* means that both the `MX` RRset and the MX host's
address (`A`/`AAAA`) records were DNSSEC-validated. The `TLSA` records are
looked up against the MX hostname (the DANE reference identifier), not the
envelope/routing domain.

| DNSSEC chain to MX host | `TLSA` lookup result | Effective `enable_tls` | Outcome |
|---|---|---|---|
| Not secure | *(not queried)* | your configured value | DANE does not apply; MTA-STS may still apply |
| Secure | usable DANE-TA(2)/DANE-EE(3) record(s) | `Required`, with the server certificate checked against the `TLSA` records | deliver only if the certificate matches a record; otherwise defer and try the next host |
| Secure | record(s) present but none usable | `RequiredInsecure` | STARTTLS required, but the server certificate is **not** checked |
| Secure | securely absent (NODATA / NXDOMAIN) | your configured value | DANE does not apply; MTA-STS may still apply |
| Secure | lookup failed (`SERVFAIL`, timeout, or bogus) | — | delivery is deferred (downgrade resistance); nothing is sent in the clear |

Notes:

* When usable DANE records are found (the `Required` row), MTA-STS is **not**
  consulted; DANE authentication wins. When records are published but none are
  usable (the `RequiredInsecure` row) the domain has no usable DANE policy, so
  MTA-STS may still be consulted to add authentication — but it cannot relax the
  mandatory STARTTLS.
* "usable" means a DANE-TA(2) or DANE-EE(3) usage with a recognized selector and
  matching type. PKIX-TA(0), PKIX-EE(1), and private/unassigned usages are
  treated as unusable.
* [`Required`](enable_tls.md) and [`RequiredInsecure`](enable_tls.md) differ
  only in whether the server certificate is checked; `RequiredInsecure` still
  mandates STARTTLS, it just does not check the peer's certificate.
* On the `RequiredInsecure` row the certificate is not validated, so by default
  SMTP AUTH PLAIN is not sent; see
  [allow_smtp_auth_plain_without_valid_certificate](allow_smtp_auth_plain_without_valid_certificate.md).

The outcome of each of these decisions is counted by the
[dane_result_count](../../metrics/kumod/dane_result_count.md) metric, which also
includes guidance on confirming that DANE is working and what to alert on.

Use of DANE *requires* a DNSSEC-validating DNS resolver; a resolver that does
not validate will treat every destination as not secure, and DANE will silently
never engage (see the
[dane_result_count](../../metrics/kumod/dane_result_count.md) metric).

You can use either the `libunbound` resolver or, {{since('dev', inline=True)}},
the Hickory resolver with validation enabled.

With `libunbound`:

```lua
kumo.on('init', function()
  kumo.dns.configure_unbound_resolver {
    options = {
      -- Enable DNSSEC
      validate = true,
    },
    -- By default, if you omit `name_servers`, unbound will
    -- resolve via the root resolvers.
    -- We strongly recommend deploying local caching nameservers
    -- and referencing them here:
    -- name_servers = { '1.1.1.1:53' },
  }
end)
```

With Hickory {{since('dev', inline=True)}}:

```lua
kumo.on('init', function()
  kumo.dns.configure_resolver {
    Hickory = {
      name_servers = {
        -- The upstream must support DNSSEC and be reachable over TCP.
        '1.1.1.1:53',
      },
      options = {
        -- Enable DNSSEC validation
        validate = true,
      },
    },
  }
end)
```

!!! note
    DNSSEC validation requires TCP: `DNSKEY`/`RRSIG` responses frequently
    exceed what fits in a UDP datagram, and a truncated response cannot be
    validated. The simple `'IP:PORT'` name server form (and the
    `udp_then_tcp` protocol) provide TCP fallback automatically; do not pin a
    validating resolver's name server to `protocol = 'udp'`, or every lookup
    will be reported as bogus.

## Trust anchors

DNSSEC validation is rooted in the DNS root zone's trust anchors, so keeping
them current is an important part of operating DANE: if your resolver's anchors
fall out of date — for example after an
[ICANN root KSK rollover](https://www.icann.org/resources/pages/ksk-rollover) —
validation begins to fail and DANE stops engaging.

Both backends ship with the current root anchors bundled, refreshed when you
upgrade KumoMTA, so the default needs no action until a future rollover. For
unattended long-term currency, point the unbound backend at an RFC 5011 managed
anchor file, which is maintained automatically across rollovers. See
[trust_anchor_file](../../kumo.dns/resolver_options/trust_anchor_file.md) for the
static and managed forms and their tradeoffs.
