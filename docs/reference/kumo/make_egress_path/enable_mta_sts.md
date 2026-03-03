# enable_mta_sts

{{since('2023.11.28-b5252a41')}}

When set to `true` (which is the default), the
[MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461) policy for the
destination domain will be used to adjust the effective value of `enable_tls`.
You can set it to `false` to disable querying of the MTA-STS policy.

When set to `true`, we'll issue additional DNS and HTTP requests appropriate
for the destination domain.  These will be cached so that we won't perform them
on every request.  For example, for `gmail.com` we'll issue a TXT lookup for
`_mta-sts.gmail.com` and an HTTP GET for
`https://mta-sts.gmail.com/.well-known/mta-sts.txt` as described in the MTA-STS
RFC.  The latter resource returns the MTA-STS polocy, which at the time of writing
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

If the `mode` for the destination domain is set to `"enforce"`, then, assuming
that the candidate MX host name matches the `mx` fields, the connection will be made
with `enable_tls="Required"`.  If the host name does not match, the candidate
MX host will be not be used.

If the `mode` is set to `"testing"`, then the connection will be made
with `enable_tls="OpportunisticInsecure"`.

If the `mode` is set to `"none"`, then your configured value for `enable_tls`
will be used.

If `enable_dane=true` and `TLSA` records are present, then any MTA-STS policy
will be ignored.


