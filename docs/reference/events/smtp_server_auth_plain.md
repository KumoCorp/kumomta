# smtp_server_auth_plain

```lua
kumo.on(
  'smtp_server_auth_plain',
  function(authz, authc, password, conn_meta) end
)
```

Called by the ESMTP server in response to the client issuing an `"AUTH PLAIN"`
authentication attempt.

KumoMTA will only allow `AUTH PLAIN` once STARTTLS has been successfully
enabled for the session.

At the time of writing KumoMTA doesn't provide a general authentication
solution, but through the use of this callback, you have some flexibility.

The event handler receives the following parameters:

* *authz* - the *authorization identity* which the client wishes to act as
* *authc* - the *authentication identity* which identifies the client for the
  purposes of validating who the client claims to be.  *authc* is paired
  with the *password* parameter.
* *password* - the password which belongs to the claimed *authc*
* *conn_meta* - represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

{{since('2023.08.22-4d895015', indent=True)}}
    The *conn_metadata* parameter is new as of this release.

Note that [PLAIN SASL](https://www.rfc-editor.org/rfc/rfc4616) allows for *authz*
to be empty.  KumoMTA will assume the same value as *authc* in that case, so
this event will always be triggered with that value.

The SMTP server expects the event handler to return a bool value; if it returns
true then it considers the credentials to be valid and will associated the claimed
identities with the session, and yield an SMTP `235` successful authentication
response to the client.  The *authz* and *authc* parameters will be set
in the message meta object as `"authz_id"` and `"authn_id"` respectively.

If it returns false then the authentication attempt is considered to have failed
and will yield an SMTP `535` failed authentication response to the client.

This example shows how to implement a very simple inline password database
using a lua table:

```lua
-- Use this to lookup and confirm a user/password credential
kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta)
  -- This is just an example of how to populate the return value,
  -- not a recommended way to handle passwords in production!
  -- In particular, it is an absolutely terrible idea to hard code
  -- a password here in plain text!
  local password_database = {
    ['daniel'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[authc] == password
end)
```

## Returning Group and identity Information

{{since('dev')}}

Rather than just returning a boolean to indicate whether authentication was
successful, you may now return an [AuthInfo](../kumo.aaa/auth_info.md) object
holding additional information.  Here's an expanded version of the example
above that shows how you can return group membership:

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
```
