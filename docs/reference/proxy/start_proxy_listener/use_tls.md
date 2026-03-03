# use_tls

{{since('dev')}}

If true, the listener will start with TLS enabled and require clients to
connect using TLS.

When TLS is enabled, you can optionally specify [tls_certificate](tls_certificate.md)
and [tls_private_key](tls_private_key.md). If not specified, a self-signed
certificate will be generated automatically.

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  use_tls = true,
}
```

