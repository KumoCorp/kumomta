# KeySource Object

KeySource objects are used in several places:

* Embedded into DKIM signer objects.
* The `tls_certificate` and `tls_key` fields of listeners.
* To hold credentials for [SMTP AUTH](./kumo/make_egress_path/smtp_auth_plain_password.md).
* With the [kumo.secrets.load](kumo.secrets/load.md) function.

## Acceptable Values

KeySources pattern match from one of the following shapes:

### Local File

When the value is a simple string value, it is interpreted as
the path to a file from which the key will be loaded when needed:

```lua
local file_signer = kumo.dkim.rsa_sha256_signer {
  key = '/path/to/file.pem',
}
```

### Caller Provided Data

!!! note
    Please also take a look at the callback/event based example further below

When the value is a table with the field `key_data`,
the value of the `key_data` field will be used as the key
data when needed:

```lua
local file_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    -- Doing literally this is probably unwise;
    -- see the example below for a more practical
    -- and manageable way to use this
    key_data = '-----BEGIN RSA PRIVATE KEY----....',
  },
}
```

`key_data` exists to allow you to manage loading key data
via some other lua function, for example, you could load
your keys from a sqlite database:

```lua
function get_key(domain, selector)
  local db = sqlite:open '/opt/kumomta/etc/dkim/keys.db'
  local result = db:execute(
    'select data from keys where domain=? and selector=?',
    domain,
    selector
  )
  return result[1]
end

local sqlite_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    key_data = get_key(msg:from_header().domain, 'default'),
  },
}
```

### HashiCorp Vault

You may store and manage your keys in a [HashiCorp
Vault](https://www.hashicorp.com/products/vault):

```lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN
    -- Note that these environment vars must be accessible
    -- by the kumod user.  If using systemd, edit the systemd
    -- service file. [Look here](docs/tutorial/using_vault/) for more information

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"

    -- Optional: specify the key name within the vault secret
    -- {{since('2025.10.06-5ec871ab', inline=True)}}
    -- Defaults to "key" if not specified
    -- vault_key = "my_custom_key_name"
  },
}
```

The key must be stored under the `path` specified. By default, it looks for a field named `key` in the vault secret.
For example, you might populate it like this:

```console
$ vault kv put -mount=secret dkim/example.org key=@example-private-dkim-key.pem
```

If you want to use a different field name, you can specify it with `vault_key` {{since('2025.10.06-5ec871ab', inline=True)}}:

```lua
local vault_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    vault_mount = 'secret',
    vault_path = 'dkim/' .. msg:from_header().domain,
    vault_key = 'private_key', -- Look for 'private_key' instead of 'key'
  },
}
```

And store it in vault like this:

```console
$ vault kv put -mount=secret dkim/example.org private_key=@example-private-dkim-key.pem
```

### Callback/Event based Data Source

{{since('2025.12.02-67ee9e96')}}

You may define an event handling function to perform parameterized data
fetching.  This is in some ways similar to using the `key_data` form described
above, but has the advantage of allowing deferred rather than eager access to
the data.

For example, you could enhance the sqlite dkim example above to use this form;
the advantage is that the cache keys become smaller and more efficient, and the
key source object is then a bit more declarative:

```lua
-- Define the event handling callback. The name must match up to the
-- event_name field used in the keysource object below
kumo.on('fetch-my-domain-key', function(domain, selector)
  local db = sqlite:open '/opt/kumomta/etc/dkim/keys.db'
  local result = db:execute(
    'select data from keys where domain=? and selector=?',
    domain,
    selector
  )
  -- The callback must return a string which, depending on
  -- the code that is consuming the keysource, may be binary.
  -- DKIM keys are PEM encoded ASCII.
  return result[1]
end)

local sqlite_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    -- name of the event handling callback you
    -- have registered via kumo.on.
    event_name = 'fetch-my-domain-key',
    -- The list of parameters that will be passed to
    -- the event handling callback
    event_args = { msg:from_header().domain, 'default' },
  },
}
```

