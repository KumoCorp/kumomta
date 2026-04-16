# tls_certificate

{{since('2025.10.06-5ec871ab')}}

Specify the path to a TLS certificate file to use for presenting the client side certificate during delivery of emails.

If unspecified, no client side certificate would be presented.

Preferred to use X.509 version 3 to support both openssl and rustls.

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
      -- ..
      tls_certificate = '/path/to/cert.pem',
    }
  end
)
```

You may specify that the certificate be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
      -- ..
      tls_certificate = {
        vault_mount = 'secret',
        vault_path = 'tls/mail.example.com.cert',

        -- Specify how to reach the vault; if you omit these,
        -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

        -- vault_address = "http://127.0.0.1:8200"
        -- vault_token = "hvs.TOKENTOKENTOKEN"
      },
    }
  end
)
```

The key must be stored as `key` (even though this is a certificate!) under the
`path` specified.  For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert key=@mail.example.com.cert
```
