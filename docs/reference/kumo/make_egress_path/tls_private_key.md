# tls_private_key

{{since('2025.10.06-5ec871ab')}}

Specify the path to the TLS private key file that corresponds to the `tls_certificate`.

If unspecified, no client side certificate would be presented.

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
      -- ..
      tls_private_key = '/path/to/key.pem',
    }
  end
)
```

You may specify that the key be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
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
  end
)
```

The key must be stored as `key` under the `path` specified.
For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com key=@mail.example.com.key
```
