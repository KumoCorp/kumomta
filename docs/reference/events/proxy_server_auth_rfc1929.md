# proxy_server_auth_rfc1929

{{since('dev')}}

```lua
kumo.on(
  'proxy_server_auth_rfc1929',
  function(username, password, conn_meta) end
)
```

Called by the proxy server when a client attempts RFC 1929 username/password
authentication.

!!! note
    This event is only available to the `proxy-server` executable.

This event is triggered when [require_auth](../kumo/start_proxy_listener/require_auth.md)
is enabled on the proxy listener, or when a client offers username/password
authentication even if not required.

The event handler receives the following parameters:

* *username* - the username provided by the client
* *password* - the password provided by the client
* *conn_meta* - a table containing connection metadata:
    * `peer_address` - the client's socket address (IP and port)
    * `local_address` - the server's socket address (IP and port)

The proxy server expects the event handler to return either a bool value or
an `AuthInfo` value.

If it returns `true` then it considers the credentials to be valid and will
allow the client to use the proxy. If it returns `false` then the
authentication attempt is considered to have failed and the connection will
be closed.

This example shows how to implement a simple inline password database:

```lua
kumo.on('proxy_server_auth_rfc1929', function(username, password, conn_meta)
  -- This is just an example of how to populate the return value,
  -- not a recommended way to handle passwords in production!
  local password_database = {
    ['proxyuser'] = 'secretpassword',
  }
  if password == '' then
    return false
  end
  return password_database[username] == password
end)
```

## Returning Group and Identity Information

Rather than just returning a boolean to indicate whether authentication was
successful, you may return an [AuthInfo](../kumo.aaa/auth_info.md) object
holding additional information. Here's an expanded version of the example
above that shows how you can return group membership:

```lua
local password_database = {
  ['proxyuser'] = {
    password = 'secretpassword',
    groups = { 'proxy-users', 'premium-tier' },
  },
}

kumo.on('proxy_server_auth_rfc1929', function(username, password, conn_meta)
  local entry = password_database[username]
  if not entry then
    return false
  end
  if entry.password ~= password then
    return false
  end

  -- Return an AuthInfo that lists the identity and group membership
  return {
    identities = {
      { identity = username, context = 'ProxyAuthRfc1929' },
    },
    groups = entry.groups,
  }
end)
```

