# iprev_msg

{{since('2025.12.02-67ee9e96')}}

```lua
local mail_auth = require 'policy-extras.mail_auth'
local auth_result = mail_auth.iprev(MSG)
```

The `mail_auth.iprev` function performs the `iprev` authentication method [as
specified by RFC8601 Section
3](https://datatracker.ietf.org/doc/html/rfc8601#section-3), and returns an
[AuthenticationResult](../authenticationresult.md) representing the status of
the check.

The `MSG` parameter is a [Message](../message/index.md) object.

See [mail_auth.iprev](iprev.md) for a version of this check at accepts
an IP address without needing a message object.

