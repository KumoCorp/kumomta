# smtp_server_auth_oauthbearer

```lua
kumo.on('smtp_server_auth_oauthbearer', function(authz, token, conn_meta) end)
```

{{since('dev')}}

Called by the ESMTP server in response to the client issuing an
`"AUTH OAUTHBEARER"` ([RFC 7628](https://www.rfc-editor.org/rfc/rfc7628))
authentication attempt.

`OAUTHBEARER` is only advertised in the `EHLO` response, and only accepted,
when a handler for this event has been registered, and only once STARTTLS
has been successfully enabled for the session.

KumoMTA doesn't validate the token itself; this event is responsible for
validating it, for example by calling the token introspection endpoint of your
identity provider.

The event handler receives the following parameters:

* *authz* - the *authorization identity* which the client wishes to act as.
  This is optional in OAUTHBEARER and may be an empty string.
* *token* - the OAuth 2.0 bearer token presented by the client
* *conn_meta* - represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

The return value is interpreted in the same way as for
[smtp_server_auth_plain](smtp_server_auth_plain.md): `true`, `false` or an
[AuthInfo](../kumo.aaa/auth_info.md) object. Since the authenticated identity
is held by the token rather than by the client response, returning `true`
requires that the client sent a non-empty *authz*, which is then used as both
the `"authz_id"` and `"authn_id"`. Prefer returning an `AuthInfo` with an
identity taken from the validated token, using the
`SmtpAuthOAuthBearerAuthentication` and `SmtpAuthOAuthBearerAuthorization`
contexts.

If the token is rejected, the client is sent the `334` error challenge
`{"status":"invalid_token"}` defined by RFC 7628, and once the client has
responded to that, an SMTP `535` failed authentication response.

The successful mechanism name is set in the connection and message metadata as
`"auth_mech"`.

## Validating tokens

This example validates the token using
[RFC 7662 token introspection](https://www.rfc-editor.org/rfc/rfc7662), caching
the result for up to a minute. It applies the resource server checks from
[RFC 9700](https://www.rfc-editor.org/rfc/rfc9700): the token must be active,
issued for this service and carry the expected scope.

Tokens bound to a DPoP key (`cnf.jkt`) are rejected: SASL has no way to carry
the DPoP proof. Tokens bound to a client certificate
([RFC 8705](https://www.rfc-editor.org/rfc/rfc8705)) are checked against the
`tls_peer_cert_sha256` connection metadata, which requires a listener
configured with
[tls_required_client_ca](../kumo/start_esmtp_listener/tls_required_client_ca.md).

```lua
local INTROSPECTION_URL =
  'https://idp.example.com/realms/mail/protocol/openid-connect/token/introspect'
local AUDIENCE = 'smtp.example.com'
local SCOPE = 'smtp.send'

local http_client = kumo.http.build_client {}

local introspect = kumo.memoize(function(token)
  local response = http_client
    :post(INTROSPECTION_URL)
    -- Don't hard code the client secret in production!
    :basic_auth('kumomta', 'client-secret')
    :form_url_encoded({ token = token })
    :send()
  if not response:status_is_success() then
    error(
      string.format(
        'token introspection failed: %d %s',
        response:status_code(),
        response:status_reason()
      )
    )
  end
  return kumo.serde.json_parse(response:text())
end, {
  name = 'oauth_introspection',
  ttl = '1 minute',
  capacity = 10000,
})

local function contains(list, value)
  if type(list) == 'string' then
    return list == value
  end
  for _, item in ipairs(list or {}) do
    if item == value then
      return true
    end
  end
  return false
end

local function validate_token(token, conn_meta)
  local claims = introspect(token)
  if not claims.active or (claims.exp and claims.exp <= os.time()) then
    return nil
  end
  if not contains(claims.aud, AUDIENCE) then
    return nil
  end
  if not contains(kumo.string.split(claims.scope or '', ' '), SCOPE) then
    return nil
  end
  local cnf = claims.cnf or {}
  if cnf.jkt then
    return nil
  end
  if
    cnf['x5t#S256']
    and cnf['x5t#S256'] ~= conn_meta:get_meta 'tls_peer_cert_sha256'
  then
    return nil
  end
  return claims
end

kumo.on('smtp_server_auth_oauthbearer', function(authz, token, conn_meta)
  local claims = validate_token(token, conn_meta)
  if not claims then
    return false
  end
  if authz ~= '' and authz ~= claims.email then
    return false
  end
  return {
    identities = {
      {
        identity = claims.email,
        context = 'SmtpAuthOAuthBearerAuthentication',
      },
    },
  }
end)
```

See [smtp_server_auth_xoauth2](smtp_server_auth_xoauth2.md) for using the same
validation for clients that use `XOAUTH2`.
