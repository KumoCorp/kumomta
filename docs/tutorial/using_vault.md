# Using HashiCorp Vault

## What is HCP Vault and why do I care?

[HashCorp Vault](https://developer.hashicorp.com/vault) is a secure storage tool for maintaining a centralized and secure store of passwords, certificates and other secrets. Vault is only one of the ways you can store secrets outside of your running code, but it is very popular and KumoMTA has [native integration](https://docs.kumomta.com/reference/keysource/?h=hashi#hashicorp-vault).  Vault helps you keep password and other secrets separated from running code to help reduce the possibility of security leaks such as accidentally saving your API key in a GitHub repo.  Oops.


## How do I configure it to work with KumoMTA?

The documentation [here](https://docs.kumomta.com/reference/keysource/?h=hashi#hashicorp-vault) is fairly straightforward, but does have some nuance. 
In the example shown there and below,we are storing the DKIM signing key as a file in vault so it can be called dynamically (more on that later), but including the vault token in the script is not particularly a secure way of doing things.  It is much better to place that addres and token in environment variables that are accessible to KumoMTA. In most cases, that will mean modifying the systemd unit service file.   

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

To modify the systemd service file, use the built in edit command in systemctl.  The [man page is here](https://man7.org/linux/man-pages/man1/systemctl.1.html), but Digital Ocean has a really good [tutorial](https://www.digitalocean.com/community/tutorials/how-to-use-systemctl-to-manage-systemd-services-and-units) that explains it in plain english.

The short version is that you can use `systemctl edit` to edit the file and add
"Environment" values under the `[Service]` section so that those values will be
available when the system service daemon starts KumoMTA. The example below
modified the FULL service config.  you can remove the --full option to only
modify a snippet as well.

```bash
sudo systemctl edit --full kumomta.service
```

You should ignore (leave it alone) everything except the `[Service]` section.
At the bottom of that section, add 2 lines:

```
 Environment=VAULT_ADDR='http://<YOUR_SERVER_LOCATION>:8200'
 Environment=VAULT_TOKEN='<YOUR_ACCESS_TOKEN>'
```

When done, it should look something like this: 

```
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
 Environment=VAULT_TOKEN='hvs.RGTlkIUYif34pjsf90wGCp0Q'

 [Install]
 WantedBy=multi-user.target
```

Save the file, then reload with `sudo systemctl daemon-reload` and the changes shoudl be immediate.


## Storing secrets for later use

There are a number of ways to store secrets, and the method depends on how you have created your Vault in the first place.  If you configured with the Vault CLI, then you can store a V2 password like this:
```bash
vault kv put -mount=secret dkim/example.org key=@example-private-dkim-key.pem
```
It is important to ensure you are storing Version-2 secrets with a "key=<value>" format.  In the command above, the `key` points to a filename `example-private-dkim-key.pem`.


## What can you do with Vault?

Vault has a number of advantages over statically storing secrets. Aside from the obvious security benefits of not exposing your passwords and security keys in your code, it allows you to PHYSICALLY separate the inforamtion as well.  One important use case is where you can havs your vault server located in your own private network while your MTAs can be deployed around the world or in public colocation or cloud services.  If one of those remote servers is compromized, you can easily secure your local vault server to protect from leakage of those secrets.

Another advantage is being able to dynamically load keys when needed.  This can be very helpful with DKIM key rotation processes. With KumoMTA, you can set up your keys in DNS in advance, then switch them at will in the config. The Lua snippet below will dynamically load the signing key based on the `from header` domain.

```Lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,
  },
} 
```

You could easily modify that code to pull a different signing key based on any number of factors.


