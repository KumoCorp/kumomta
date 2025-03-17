# enable_mta_sts

{{since('2023.11.28-b5252a41')}}

When set to `true` (which is the default), the
[MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461) policy for the
destination domain will be used to adjust the effective value of `enable_tls`.

If the policy is set to `"enforce"`, then, assuming that the candidate
MX host name matches the policy, the connection will be made with
`enable_tls="Required"`.  If the host name does not match, the candidate
MX host will be not be used.

If the policy is set to `"testing"`, then the connection will be made
with `enable_tls="OpportunisticInsecure"`.

If the policy is set to `"none"`, then your configured value for `enable_tls`
will be used.

If `enable_dane=true` and `TLSA` records are present, then any MTA-STS policy
will be ignored.


