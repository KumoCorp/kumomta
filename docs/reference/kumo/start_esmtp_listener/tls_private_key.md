# tls_private_key

Specify the path to the TLS private key file that corresponds to the `tls_certificate`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

{{since('2025.05.06-b29689af', indent=True)}}
    The private key will be cached for 5 minutes, then re-evaluated,
    allowing for the privae key to be updated without restarting
    the service. In prior versions of KumoMTA you would need to
    restart kumod in order to pick up an updated private key.


```lua
kumo.start_esmtp_listener {
  -- ..
  tls_private_key = '/path/to/key.pem',
}
```

You may specify that the key be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_private_key = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.key',

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` under the `path` specified.
For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com key=@mail.example.com.key
```


