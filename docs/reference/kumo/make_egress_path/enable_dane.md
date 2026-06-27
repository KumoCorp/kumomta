# enable_dane

{{since('2023.11.28-b5252a41')}}

When set to `true` (the default is `false`), then `TLSA` records will be
resolved securely to determine the destination site policy for TLS according
to [DANE](https://datatracker.ietf.org/doc/html/rfc7672).

The table below applies when `enable_dane = true`. A *secure chain* means the
MX host selection is trusted — either the `MX` RRset was DNSSEC-validated, or the
host was supplied via a locally-configured `mx_list` for which you set
[treat_mx_list_as_secure](../make_queue_config/protocol.md#treat_mx_list_as_secure)
— **and** the MX host's address (`A`/`AAAA`) records were DNSSEC-validated. The
`TLSA` records are looked up against the MX hostname (the DANE reference
identifier), not the envelope/routing domain. A destination given as an IP
address literal is never DANE-eligible (there is no name to query).

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

## Limitations

### MX hosts that are CNAMEs into unsigned zones

When a destination's MX host is a `CNAME` whose target lands in an **unsigned**
(non-DNSSEC) zone, KumoMTA does not engage DANE for that host, even if `TLSA`
records are published securely at the original MX name. The MX host's address
(`A`/`AAAA`) records resolve through the unsigned zone, so the chain is not
fully secure and the delivery follows the first row of the table above: DANE
does not apply and the configured [enable_tls](enable_tls.md) value is used.

This fails safe. KumoMTA never falsely passes DANE and never downgrades a
DANE-eligible host to cleartext; the only effect is *missed* DANE pinning, and
the case is uncommon. Because the destination's DNS layout is outside the
sender's control, there is no sender-side change that makes DANE engage here.

#### Recognizing this case with `dig`

The signature is an MX host whose address lookup is a `CNAME` *into a zone that
is not signed*, while `TLSA` records are still published (signed) at the MX
name. Query the MX host's address with `+dnssec` and watch the `ad`
(Authenticated Data) flag and the `RRSIG` records:

```console
$ dig +dnssec mx.example.com. A

;; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: 12345
;; flags: qr rd ra ad; QUERY: 1, ANSWER: 3, AUTHORITY: 0, ADDITIONAL: 1

;; ANSWER SECTION:
mx.example.com.      3600 IN CNAME  mail.provider.net.
mx.example.com.      3600 IN RRSIG  CNAME 13 3 3600 ( ... )   ; the CNAME is signed
mail.provider.net.    300 IN A      192.0.2.25                ; target: no RRSIG
```

Note that the `CNAME` carries an `RRSIG` (it lives in a signed zone) but the
target `A` record does **not** — `mail.provider.net` is an unsigned zone. A
validating resolver therefore cannot set the `ad` flag for the address as a
whole, even though `ad` *is* present here for the signed `CNAME` step. The
matching `TLSA` records still validate at the original MX name:

```console
$ dig +dnssec _25._tcp.mx.example.com. TLSA

;; flags: qr rd ra ad; ...

;; ANSWER SECTION:
_25._tcp.mx.example.com. 3600 IN TLSA  3 1 1 ( ... )
_25._tcp.mx.example.com. 3600 IN RRSIG TLSA 13 4 3600 ( ... )
```

If your destination looks like this — a signed `CNAME` whose target `A`/`AAAA`
records are unsigned, plus signed `TLSA` records at the MX name — it falls under
this limitation and KumoMTA will not engage DANE for it. If instead the target
zone is signed (the `A`/`AAAA` records carry their own `RRSIG` and the address
lookup is fully `ad`), DANE engages normally and no action is needed.

For the typical sender-focused deployment this needs no attention: the default
[`Opportunistic`](enable_tls.md) TLS still provides **privacy** (encryption)
for these deliveries, if not authentication, which is usually sufficient for
this kind of workload.

If you have an **explicit requirement** for authenticated TLS to such a
destination, you can:

* Set [`enable_tls = "Required"`](enable_tls.md) **for that specific
  destination** so delivery fails closed rather than dropping to opportunistic.
  Note that this authenticates the peer against the WebPKI, not against a `TLSA`
  pin.
* For stronger guarantees, deploy an explicit `mx_list` for the destination
  with
  [treat_mx_list_as_secure](../make_queue_config/protocol.md#treat_mx_list_as_secure)
  set to `true`, asserting out-of-band operator trust in the destination hosts
  instead of relying on DNS.

Setting [`Required`](enable_tls.md) *globally* is **not** recommended for sender
deployments: TLS support across arbitrary destinations is inconsistent, and a
blanket policy causes delivery failures rather than improving security.

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
