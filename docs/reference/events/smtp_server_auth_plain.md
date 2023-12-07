# `kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta))`

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
  local password_database = {
    ['scott'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[authc] == password
end)
```
