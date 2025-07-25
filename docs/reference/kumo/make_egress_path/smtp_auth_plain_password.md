# smtp_auth_plain_password

Specifies the password that should be used together with `smtp_auth_plain_username`
when an authenticated SMTP connection is desired.

The value is any [keysource](../../keysource.md), which allows for specifying the
password inline in the configuration file, or managing it via a credential manager
such as HashiCorp Vault.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'daniel',
    -- The password can be any keysource value.
    -- Here we are loading the credential for the domain
    -- from HashiCorp vault
    smtp_auth_plain_password = {
      vault_mount = 'secret',
      vault_path = 'smtp-auth/' .. domain,
      -- Optional: specify a custom key name (defaults to "key")
      -- vault_key = "password"
    },
  }
end)
```


