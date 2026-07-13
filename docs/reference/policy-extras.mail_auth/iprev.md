# iprev

{{since('2025.12.02-67ee9e96')}}

```lua
local mail_auth = require 'policy-extras.mail_auth'
local auth_result = mail_auth.iprev(IP)
```

The `mail_auth.iprev` function performs the `iprev` authentication method [as
specified by RFC8601 Section
3](https://datatracker.ietf.org/doc/html/rfc8601#section-3), and returns an
[AuthenticationResult](../authenticationresult.md) representing the status of
the check.

The `IP` parameter is a string representation of the IP address; for example,
`"127.0.0.1"` for an IPv4 address of `"::1"` for an IPv6 address.

See [mail_auth.iprev_msg](iprev_msg.md) for a version of this check at accepts
a [Message](../message/index.md) object instead.
