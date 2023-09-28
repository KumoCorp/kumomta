# Using HashiCorp Vault

## Introduction

[HashCorp Vault](https://developer.hashicorp.com/vault) is a secure storage tool for maintaining a centralized and secure store of passwords, certificates and other secrets. Vault is only one of the ways you can store secrets outside of your running code, but it is very popular and KumoMTA has [native integration](https://docs.kumomta.com/reference/keysource/?h=hashi#hashicorp-vault).

Vault helps you keep password and other secrets separated from running code to help reduce the possibility of security leaks such as accidentally saving your API key in a GitHub repo.

## Configuring KumoMTA to use Hashicorp Vault

The documentation [in the reference manual](https://docs.kumomta.com/reference/keysource/?h=hashi#hashicorp-vault) is straightforward, but does have some nuance.

In the example shown there and below,we are storing the DKIM signing key as a file in vault so it can be called dynamically, but including the vault token in the script is not a particularly secure way of doing things. It is recommended to place the vault address and token in environment variables that are accessible to KumoMTA. In most cases, that will mean modifying the systemd unit service file.

```lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,
    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

To modify the systemd service file, use the built in edit command in systemctl. The [man page is here](https://man7.org/linux/man-pages/man1/systemctl.1.html), but Digital Ocean has an excellent [tutorial](https://www.digitalocean.com/community/tutorials/how-to-use-systemctl-to-manage-systemd-services-and-units) that explains it in plain english.

The short version is that you can use `systemctl edit` to edit the file and add "Environment" values under the `[Service]` section so that those values will be available when the system service daemon starts KumoMTA. The example below modified the FULL service config. The --full option can be remove to modify a snippet instead of the full config.

```bash
sudo systemctl edit --full kumomta.service
```

You should disregard everything except the `[Service]` section.
At the bottom of that section, add 2 lines:

```toml
Environment=VAULT_ADDR='http://<YOUR_SERVER_LOCATION>:8200'
Environment=VAULT_TOKEN='<YOUR_ACCESS_TOKEN>'
```

When done, it should look something like this:

```toml
[Unit]
Description=KumoMTA SMTP service
After=syslog.target network.target
Conflicts=sendmail.service exim.service postfix.service

[Service]
Type=simple
Restart=always
ExecStart=/opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod
# Allow sufficient time to wrap up in-flight tasks and safely
# write out pending data
TimeoutStopSec=300
Environment=VAULT_ADDR='http://127.0.0.1:8200'
Environment=VAULT_TOKEN='SAMPLE-TOKEN'

[Install]
WantedBy=multi-user.target
```

Save the file, then reload with the `sudo systemctl daemon-reload` command.

## Storing secrets for later use

There are a number of ways to store secrets, and the method depends on how the vault was created. If configured via the Vault CLI, then aa V2 password can be stored as follows:

```bash
vault kv put -mount=secret dkim/example.org key=@example-private-dkim-key.pem
```

It is important to ensure you are storing Version-2 secrets with a "key=<value>" format. In the preceding example, the `key` points to a filename `example-private-dkim-key.pem`.

## Ways to Use Vault With KumoMTA

Vault has a number of advantages over statically storing secrets. Aside from the obvious security benefits of not exposing your passwords and security keys in your code, it also allows you to *physically* separate the information. One key use case is storing the vault server in a private network while the KumoMTA instances are deployed around the world or in public colocation or cloud services. If a remote server is compromised, the local vault server can be secured to prevent data leakage.

Another advantage is being able to dynamically load keys on demand. This can be very helpful with DKIM key rotation. With the keys stored within the vault, they can be loaded as-needed when messages pass through the server that need a particular key:

```Lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,
  },
}
```
