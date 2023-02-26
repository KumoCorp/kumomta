# `kumo.on('http_server_validate_auth_basic', function(USER, PASSWORD))`

Called by the HTTP listener to validate HTTP Basic authentication
credentials provided by the client.

At the time of writing KumoMTA doesn't provide a general authentication
solution, but through the use of this callback, you have some flexibility.

The event handler receives the username and password provided by the client.
Note that the password may be empty or missing if the client provided only
a user name.

The HTTP server expects the event handler to return a bool value; if it returns
true then it considers the credentials to be valid and will allow the client to
access the endpoint. If it returns false then it will consider the credentials
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
