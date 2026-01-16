# require_auth

If true, the proxy server will require RFC 1929 username/password authentication
from clients before allowing them to use the proxy.

When enabled, you must also register a handler for the
[proxy_server_auth_rfc1929](../../events/proxy_server_auth_rfc1929.md) event
to validate credentials.

The default is `false`.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  require_auth = true,
}

kumo.on('proxy_server_auth_rfc1929', function(username, password, conn_meta)
  -- Validate credentials
  return username == 'user' and password == 'secret'
end)
```

