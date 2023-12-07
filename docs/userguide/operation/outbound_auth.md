# Delivering Messages Using SMTP AUTH

While not used when delivering messages to remote hosts under normal circumstances, there are scenarios where the KumoMTA server must authenticate when relaying mail. Some examples include:

* Relaying incoming mail to internal hosts that require authentication.
* Relaying outgoing mail through a third-party relay provider via SMTP.
* Delivering mail to a peer system as part of a processing chain.

## Configuring an egress_path to Use AUTH

The following example shows how SMTP AUTH information can be added to an egress_path config:

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

!!!warning
    The above example would add AUTH credentials to every outbound connection. In production, this should be selectively applied based on the destination host or domain.

See the [make_egress_path](../../reference/kumo/make_egress_path.md) section of the Reference Manual for more information.

## Using a Keysource with SMTP AUTH

Storing credentials in a static policy file is not recommended. KumoMTA supports multiple options for secure key storage, and we highly recommend that all authentication and signing keys be stored in a keysource.

When using a keysource, the value of `smtp_auth_plain_password is any [keysource](../../reference/keysource.md), which allows for specifying the password via a credential manager such as HashiCorp Vault.

```lua
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Required',
    smtp_auth_plain_username = 'scott',
    -- The password can be any keysource value.
    -- Here we are loading the credential for the domain
    -- from HashiCorp vault
    smtp_auth_plain_password = {
      vault_mount = 'secret',
      vault_path = 'smtp-auth/' .. domain,
    },
  }
end)
```

See the [keysource](https://docs.kumomta.com/reference/keysource/) section of the Reference Manual for more information.

## Using The Traffic Shaping Helper with AUTH Parameters

As mentioned earlier, SMTP AUTH must be selectively applied. One way to facilitate the configuration of SMTP AUTH is to use the `shaping.lua` traffic shaping helper.

When using shaping.lua, the hostname or IP of the target host can be used as a domain, with mx_rollup disabled, and the AUTH options listed.

For example, to use a keysource with a local host, the following could be added to a custom TOML file:

```toml
["192.168.1.10"]
mx_rollup = false
smtp_auth_plain_username = "scott"
smtp_auth_plain_password = { vault_mount = "secret", vault_path = "smtp-auth/local" }
```

See the [traffic shaping](../configuration/trafficshaping.md#using-the-shapinglua-helper) section of the User Guide for additional information.
