# tls_certificate

Specify the path to a TLS certificate file to use for the server identity when
*use_tls* is set to `true`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_http_listener {
  -- ..
  tls_certificate = '/path/to/cert.pem',
}
```

You may specify that the certificate be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_http_listener {
  -- ..
  tls_certificate = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.cert',
    -- Optional: specify a custom key name (defaults to "key")
    -- vault_key = "certificate"

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The certificate must be stored under the `path` specified. By default, it looks for a field named `key` in the vault secret.
For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert key=@mail.example.com.cert
```

If you want to use a different field name, you can specify it with `vault_key` {{since('dev', inline=True)}}:

```lua
kumo.start_http_listener {
  -- ..
  tls_certificate = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.cert',
    vault_key = 'certificate', -- Look for 'certificate' instead of 'key'
  },
}
```

And store it in vault like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert certificate=@mail.example.com.cert
```


