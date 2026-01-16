# hostname

Specifies the hostname to use when generating a self-signed TLS certificate.

The default is the system hostname.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  use_tls = true,
  hostname = 'proxy.example.com',
}
```

