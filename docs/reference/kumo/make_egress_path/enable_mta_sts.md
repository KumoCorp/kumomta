# enable_mta_sts

{{since('2023.11.28-b5252a41')}}

When set to `true` (which is the default), a resolved
[MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461) policy for the
destination domain will be used to adjust the effective value of `enable_tls`.
You can set it to `false` to prevent a policy from raising the TLS posture for
this egress path.

{{since('dev', indent=True)}}
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

If the `mode` for the destination domain is set to `"enforce"`, then the
connection will be made with `enable_tls="Required"`. Candidate MX hosts that do
not match the `mx` fields are removed during resolution, so by the time a
connection is attempted the candidate set already satisfies the policy.

If the `mode` is set to `"testing"`, then the connection will be made
with `enable_tls="OpportunisticInsecure"`.

If the `mode` is set to `"none"`, then your configured value for `enable_tls`
will be used.

If `enable_dane=true` and `TLSA` records are present, then any MTA-STS policy
will be ignored.
