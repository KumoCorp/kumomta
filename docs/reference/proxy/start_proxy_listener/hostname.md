# hostname

{{since('2026.03.04-bb93ecb1')}}

Specifies the hostname to use when generating a self-signed TLS certificate.

The default is the system hostname.

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  use_tls = true,
  hostname = 'proxy.example.com',
}
```

