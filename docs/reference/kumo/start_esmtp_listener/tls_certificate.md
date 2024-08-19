# tls_certificate

Specify the path to a TLS certificate file to use for the server identity when
the client issues `STARTTLS`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_certificate = '/path/to/cert.pem',
}
```

You may specify that the certificate be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_esmtp_listener {
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
```

The key must be stored as `key` (even though this is a certificate!) under the
`path` specified.  For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert key=@mail.example.com.cert
```


