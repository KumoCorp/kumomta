# smtp_auth_plain_username

When set, connecting to the destination requires a successful AUTH PLAIN using the
specified username.

AUTH PLAIN will only be attempted if TLS is also enabled, unless
`allow_smtp_auth_plain_without_tls = true`. This is to prevent leaking
of the credential over an unencrypted link.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value
    smtp_auth_plain_password = {
      key_data = 'tiger',
    },
  }
end)
```


