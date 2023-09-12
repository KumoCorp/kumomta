# `kumo.on('http_server_validate_auth_basic', function(USER, PASSWORD))`

Called by the HTTP listener to validate HTTP Basic authentication
credentials provided by the client.

At the time of writing KumoMTA doesn't provide a general authentication
solution, but through the use of this callback, you have some flexibility.

The event handler receives the username and password provided by the client.
Note that the password may be empty or missing if the client provided only
a user name.

The HTTP server expects the event handler to return a bool value; if it returns
`true` then it considers the credentials to be valid and will allow the client to
access the endpoint. If it returns `false` then it will consider the credentials
to be invalid and return a authorization error. Other return values, or raising
an error, will return an error status to the client.

This example shows how to implement a very simple inline password database
using a lua table:

```lua
-- Use this to lookup and confirm a user/password credential
-- used with the http endpoint
kumo.on('http_server_validate_auth_basic', function(user, password)
  local password_database = {
    ['scott'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[user] == password
end)
```

## Reasoning about the authorized identity

When using auth to grant access to the HTTP injection API, the authorization
identity will be made available in the generated message by setting the
`http_auth` meta key.  It can have one of the following values:

* When HTTP Basic auth is used (and validated via the
  `http_server_validate_auth_basic` event), it will be set to the provided
  username
* When no HTTP auth is used, access is granted based on the
  [trusted_hosts](../kumo/start_http_listener.md#trusted_hosts). In this case,
  `http_auth` will be set to the peer address that matched the `trusted_hosts`

If you wish to enforce or restrict some capability based on identity, you might
use logic along the lines of:

```lua
kumo.on('http_message_generated', function(msg)
  local auth = msg:get_meta 'http_auth'
  if auth ~= 'some.one' then
    error 'only some.one is allowed to inject'
  end
end)
```
