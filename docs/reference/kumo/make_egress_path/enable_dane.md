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

* When DANE applies (the `Required` / `RequiredInsecure` rows), MTA-STS is
  **not** consulted; the DANE result wins.
* "usable" means a DANE-TA(2) or DANE-EE(3) usage with a recognized selector and
  matching type. PKIX-TA(0), PKIX-EE(1), and private/unassigned usages are
  treated as unusable.
* [`Required`](enable_tls.md) and [`RequiredInsecure`](enable_tls.md) differ
  only in whether the server certificate is checked; `RequiredInsecure` still
  mandates STARTTLS, it just does not check the peer's certificate.

Use of DANE also *requires* functioning DNSSEC in your DNS resolver; you
will need to configure the `libunbound` resolver to successfully use DANE:

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
