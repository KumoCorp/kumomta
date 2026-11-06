# smtp_server_auth_xoauth2

```lua
kumo.on('smtp_server_auth_xoauth2', function(user, token, conn_meta) end)
```

{{since('dev')}}

Called by the ESMTP server in response to the client issuing an
`"AUTH XOAUTH2"` authentication attempt.  `XOAUTH2` is the mechanism that
Google and Microsoft defined before
[OAUTHBEARER](smtp_server_auth_oauthbearer.md) was standardized, and is still
the only OAuth mechanism supported by some mail clients.

`XOAUTH2` is only advertised in the `EHLO` response, and only accepted, when a
handler for this event has been registered, and only once STARTTLS has been
successfully enabled for the session.

The event handler receives the following parameters:

* *user* - the user name sent by the client
* *token* - the OAuth 2.0 bearer token presented by the client
* *conn_meta* - represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

The return value is interpreted in the same way as for
[smtp_server_auth_plain](smtp_server_auth_plain.md): `true`, `false` or an
[AuthInfo](../kumo.aaa/auth_info.md) object.  When returning `true`, *user* is
used as both the `"authz_id"` and `"authn_id"`.  When returning an `AuthInfo`,
use the `SmtpAuthXOAuth2Authentication` and `SmtpAuthXOAuth2Authorization`
contexts.

If the token is rejected, the client is sent the `334` error challenge
`{"status":"401","schemes":"bearer"}`, and once the client has responded to
that, an SMTP `535` failed authentication response.

The successful mechanism name is set in the connection and message metadata as
`"auth_mech"`.

This example shares the `validate_token` function from the
[smtp_server_auth_oauthbearer](smtp_server_auth_oauthbearer.md#validating-tokens)
example:

```lua
kumo.on('smtp_server_auth_xoauth2', function(user, token, conn_meta)
  local claims = validate_token(token, conn_meta)
  if not claims or claims.email ~= user then
    return false
  end
  return {
    identities = {
      { identity = claims.email, context = 'SmtpAuthXOAuth2Authentication' },
    },
  }
end)
```
