# tls_private_key

{{since('dev')}}

Specify the path to a TLS private key file to use for the server identity when
*use_tls* is set to `true`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  use_tls = true,
  tls_private_key = '/path/to/key.pem',
}
```

You may specify that the key be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  use_tls = true,
  tls_private_key = {
    vault_mount = 'secret',
    vault_path = 'tls/proxy.example.com.key',
  },
}
```

