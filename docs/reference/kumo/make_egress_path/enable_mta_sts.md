# enable_mta_sts

{{since('2023.11.28-b5252a41')}}

When set to `true` (which is the default), a resolved
[MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461) policy for the
destination domain adjusts the effective value of `enable_tls` as described
below. You can set it to `false` to prevent a policy from changing the TLS
posture for this egress path.

{{since('2026.09.22-a276d4a8', indent=True)}}
    This option influences only whether the TLS portion of the MTA-STS policy
    is applied on this particular egress path.  It doesn't control whether the
    MTA-STS records are queried.  Since MTA-STS records can influence the
    effective *site_name*, they are now queried before we instantiate the
    egress path.
    [kumo.dns.set_mta_sts_enabled](../../kumo.dns/set_mta_sts_enabled.md)
    controls whether MTA-STS records are used.  In earlier versions,
    `enable_mta_sts` controlled both querying and TLS level, which could lead to
    broken routing for certain types of domains sharing the same MXs.

For example, for `gmail.com` we'll issue a TXT lookup for
`_mta-sts.gmail.com` and an HTTP GET for
`https://mta-sts.gmail.com/.well-known/mta-sts.txt` as described in the MTA-STS
RFC.  The latter resource returns the MTA-STS policy, which at the time of writing
looks like this for `gmail.com`:

```
version: STSv1
mode: enforce
mx: gmail-smtp-in.l.google.com
mx: *.gmail-smtp-in.l.google.com
max_age: 86400
```

The `mode` field describes the intended policy of the destination site, while
the `mx` fields place restrictions on the allowable list of MX hosts.

The effective value of `enable_tls` depends on the policy `mode`, the value
configured for this egress path, and the KumoMTA version:

| MTA-STS `mode` | Configured `enable_tls` | Earlier versions | {{since('dev', inline=True)}} |
| --- | --- | --- | --- |
| `enforce` | Any value | `Required` | `Required` |
| `testing` | `Opportunistic` | `OpportunisticInsecure` | `Opportunistic` |
| `testing` | `OpportunisticInsecure` | `OpportunisticInsecure` | `OpportunisticInsecure` |
| `testing` | `Required` | `OpportunisticInsecure` | `Required` |
| `testing` | `RequiredInsecure` | `OpportunisticInsecure` | `RequiredInsecure` |
| `testing` | `Disabled` | `OpportunisticInsecure` | `Disabled` |
| `none` | Any value | The configured value | The configured value |

{{since('dev', indent=True)}}
    A policy can only raise the effective `enable_tls`, never lower it. In
    earlier versions a `testing` policy replaced any configured value with the
    weaker `OpportunisticInsecure`, dropping certificate verification and
    making STARTTLS optional even on a `Required` path.

For `enforce`, MX resolution removes any host that does not match the
policy's `mx` fields, leaving only hosts the policy permits by the time a
connection is attempted.

The table describes delivery without DANE. When `enable_dane=true` and the
destination publishes usable `TLSA` records, the MTA-STS policy is ignored in
favor of DANE; when the records are present but unusable, DANE mandates
STARTTLS and a `testing` policy cannot relax it. See [enable_dane](enable_dane.md).
