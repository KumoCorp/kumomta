# AuthInfo Object

{{since('dev')}}

AuthInfo Objects are used to represent the authentication state of a session.
There are two main ways that you might interact with them in KumoMTA:

 * When performing ad-hoc authorization checks via [kumo.aaa.query_resource_access](query_resource_access.md)
 * When handling authentication checks via [smtp_server_auth_plain](../events/smtp_server_auth_plain.md) or [http_server_validate_auth_basic](../events/http_server_validate_auth_basic.md).

## AuthInfo fields

The following fields may be present in an AuthInfo object:

 * `identities` - an array style table listing each authenticated identity.  An identity is itself an object of the form `{identity = 'username', context = 'GenericAuth'}` where the context describes where the credential came from.  Context can be one of the following values:
    * `SmtpAuthPlainAuthorization` - the identity came from the SMTP AUTH PLAIN `authz` field, the authorization identity.
    * `SmtpAuthPlainAuthentication` - the identity came from the SMTP AUTH PLAIN `authc` field, the authenticated identity.
    * `HttpBasicAuth` - the identity came from an HTTP Basic auth header
    * `BearerToken` - the identity came from an HTTP Bearer token
    * `ProxyAuthRfc1929` - the identity came from a SOCKS 5 RFC 1929 authentication packet
    * `LocalSystem` - a special identity representing the system itself
    * `GenericAuth` - the identity was produced by some generic authentication processing/handling and doesn't provide any additional context on the provenance of the authenticated identity
 * `groups` - an array style table listing each group name to which the session belongs
 * `peer_address` - an optional string representing the ip address of the connected peer

## Constructing an AuthInfo

When implementing [smtp_server_auth_plain](../events/smtp_server_auth_plain.md)
or [http_server_validate_auth_basic](../events/http_server_validate_auth_basic.md),
you may optionally return an `AuthInfo` object representing the overall
identity and group membership:

```lua
-- This is just an example of how to populate the return value,
-- not a recommended way to handle passwords in production!
-- In particular, it is an absolutely terrible idea to hard code
-- a password here in plain text!

local password_database = {
  ['daniel'] = {
    password = 'tiger',
    groups = { 'group1', 'group2' },
  },
}

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  local entry = password_database[authc]
  if not entry then
    return false
  end
  if entry.password ~= password then
    return false
  end

  -- Return an AuthInfo that lists out the identity and group
  -- membership
  return {
    identities = {
      { identity = authz, context = 'SmtpAuthPlainAuthorization' },
      { identity = authc, context = 'SmtpAuthPlainAuthentication' },
    },
    groups = entry.groups,
  }
end)

kumo.on('http_server_validate_auth_basic', function(user, password)
  local entry = password_database[user]
  if not entry then
    return false
  end
  if entry.password ~= password then
    return false
  end

  -- Return an AuthInfo that lists out the identity and group
  -- membership
  return {
    identities = {
      { identity = user, context = 'HttpBasicAuth' },
    },
    groups = entry.groups,
  }
end)
```
