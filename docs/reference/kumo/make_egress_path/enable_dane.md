# enable_dane

{{since('2023.11.28-b5252a41')}}

When set to `true` (the default is `false`), then `TLSA` records will be
resolved securely to determine the destination site policy for TLS according
to [DANE](https://datatracker.ietf.org/doc/html/rfc7672).

The table below applies when `enable_dane = true`. A *secure chain* means the
MX host selection is trusted — either the `MX` RRset was DNSSEC-validated, or the
host was supplied via a locally-configured `mx_list` for which you set
[treat_mx_list_as_secure](../make_queue_config/protocol.md#treat_mx_list_as_secure)
— **and** the MX host's address (`A`/`AAAA`) records were DNSSEC-validated *or*
the host is a securely published `CNAME` whose target merely lands in an
unsigned zone (see [CNAME MX hosts](#cname-mx-hosts) below). The `TLSA` records
are looked up against the MX hostname (the DANE reference identifier), not the
envelope/routing domain. A destination given as an IP address literal is never
DANE-eligible (there is no name to query).

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

## CNAME MX hosts

When an MX host is a `CNAME`, KumoMTA looks up `TLSA` records at the **original**
MX name (RFC 7672 section 2.2.2), not the expanded target. This includes the
case where the alias target lands in an **unsigned** (non-DNSSEC) zone: provided
the `CNAME` itself is securely (DNSSEC) published, DANE still engages at the
original name, because it is the securely published `TLSA` RRset — not the
address records — that authenticates the peer.

You can recognize this shape with `dig` by querying the MX host's address with
`+dnssec` and watching the `RRSIG` records: the `CNAME` carries an `RRSIG` (it
lives in a signed zone) while the target `A`/`AAAA` records do not (the target
zone is unsigned).

```console
$ dig +dnssec mx.example.com. A

;; ANSWER SECTION:
mx.example.com.      3600 IN CNAME  mail.provider.net.
mx.example.com.      3600 IN RRSIG  CNAME 13 3 3600 ( ... )   ; the CNAME is signed
mail.provider.net.    300 IN A      192.0.2.25                ; target: no RRSIG

$ dig +dnssec _25._tcp.mx.example.com. TLSA

;; ANSWER SECTION:
_25._tcp.mx.example.com. 3600 IN TLSA  3 1 1 ( ... )
_25._tcp.mx.example.com. 3600 IN RRSIG TLSA 13 4 3600 ( ... )
```

To confirm the secure status of the alias itself, KumoMTA issues an explicit
`CNAME`-type query; that query is answered by the alias RRset and is not chased
into the unsigned target, so its DNSSEC status reflects only the alias's own
zone. If the `CNAME` is securely published, DANE engages; if the alias is not
securely published (for example the MX host's own zone is unsigned), DANE does
not apply and the configured [enable_tls](enable_tls.md) value is used.

## Limitations

### TLSA records published only at the CNAME-expanded name

KumoMTA queries `TLSA` records at the original MX name only. The one `CNAME`
shape it does **not** handle is a secure alias whose `TLSA` records are
published *solely* at the fully-expanded (canonical) name and not at the
original MX name. RFC 7672 section 2.2.3 permits trying the expanded name as an
additional `TLSA` base; KumoMTA does not, so such a destination is treated as
having no DANE policy.

This is uncommon, and like every non-engaging case it fails safe: KumoMTA never
falsely passes DANE and never downgrades a DANE-eligible host to cleartext — the
only effect is *missed* pinning, after which the configured
[enable_tls](enable_tls.md) value applies.

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
