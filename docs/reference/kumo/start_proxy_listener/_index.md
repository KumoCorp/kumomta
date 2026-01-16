# kumo.start_proxy_listener

```lua
kumo.start_proxy_listener { PARAMS }
```

Configure and start a SOCKS5 proxy server.

This function should be called only from inside your [init](../../events/init.md)
event handler.

The proxy server implements the SOCKS5 protocol and can be used by KumoMTA's
egress sources to route outbound connections through the proxy. This is useful
for scenarios where you need to control the egress IP address or route traffic
through specific network paths.

```lua
kumo.on('init', function()
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
  }
end)
```

To enable TLS and authentication:

```lua
kumo.on('init', function()
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
    use_tls = true,
    tls_certificate = '/path/to/cert.pem',
    tls_private_key = '/path/to/key.pem',
    require_auth = true,
  }
end)

kumo.on('proxy_server_auth_rfc1929', function(username, password, conn_meta)
  -- Validate credentials here
  return username == 'user' and password == 'secret'
end)
```

`PARAMS` is a lua table that can accept the keys listed below:

## Proxy Listener Parameters { data-search-exclude }

