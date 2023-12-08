# Checking Inbound SMTP Authentication

When hosting relay users it is important to protect your infrastructure from malicious senders, often without the ability to whitelist the IP addresses of legitimate users. In such environments, it is critical to setup SMTP Authentication to validate injecting hosts before relaying their mail.

!!!note
    Authentication in KumoMTA can only occur on a TLS protected connection after `STARTTLS` has successfully been processed. This is because AUTH PLAIN credentials can be decoded and should not be sent over an open connection.

## Checking Authentication Against a Static User Table

The simplest implementation of AUTH checking could be implemented by checking against a static value or table:

```lua
-- Use this to lookup and confirm a user/password credential
kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta)
  local password_database = {
    ['scott'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[authc] == password
end)
```

The preceding example, also seen on the [smtp_server_auth_plain](../../reference/events/smtp_server_auth_plain.md) page of the [Reference Manual](../../reference/index.md), simply checks against a table of usernames and passwords, looking for a match. If the password is blank the function returns false, otherwise the function returns true if the password in the table for the named user matches the password provided in the AUTH request.

## Querying a Datasource for Authentication

A common use case for relay hosts is validating AUTH credentials against a datasource for more dynamic management of sending users.

In the following example, the provided credentials are checked against a SQLite database:

```lua
local sqlite = require 'sqlite'

-- Consult a hypothetical sqlite database that has an auth table
-- with user and pass fields
function sqlite_auth_check(user, password)
  local db = sqlite.open '/path/to/auth.db'
  local result = db:execute(
    'select user from auth where user=? and pass=?',
    user,
    password
  )
  -- if we return the username, it is because the password matched
  return result[1] == user
end

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return sqlite_auth_check(authc, password)
end)
```

!!!warning
    To prevent blocking when checking data like AUTH credentials we recommend using the [Memoize](../../reference/kumo/memoize.md) function to cache query results for future connections.

## Querying a Keystore for Authentication

A more secure option for storing authentication credentials for checking is Hashicorp Vault. See the [Storing Secrets in Hashicorp Vault](./hashicorp_vault.md) page for more information on how to populate the credentials in the Vault as well as how to secure the connection credentials.

```lua
function vault_auth_check(user, password)
  return password
    == kumo.secrets.load {
      vault_mount = 'secret',
      vault_path = 'smtp-auth/' .. user,
    }
end

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return vault_auth_check(authc, password)
end)
```

## Enhancing Tenant Security Through SMTP Authentication

When using SMTP authentication a certain amount of trust is put in the injecting client, and there are ways this can be abused.

One example of this is using headers for identifying which tenant a message is associated with when using the [Queues Helper](../configuration/queuemanagement.md#using-the-queues-helper) to manage queues; you can designate a custom header that contains the tenant name, trusting the user to provide their own tenant name, but if a malicious user discovers the tenant name of another user on the server, they can spoof the other tenant.

To prevent this, you can use the `require_authz` option in the helper:

```toml
[tenant.'mytenant']
# Which pool should be used for this tenant
egress_pool = 'pool-1'

# Only the authorized identities are allowed to use this tenant via the tenant_header
#require_authz = ["scott"]
```

This prevents users other than **scott** (multiple users can be specified) from using the tenant for sending.
