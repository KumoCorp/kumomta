# kumo.secrets.load

```lua
kumo.secrets.load(SOURCE)
```

Given a [keysource](../keysource.md), load and return the bytes stored in that source.

## Example of Loading a credential from a vault

```lua
local request = kumo.http.build_client({}):get 'https://example.com/'

local passwd = kumo.secrets.load {
  vault_mount = 'secret',
  vault_path = 'example.com-passwd',
  -- Optional: specify a custom key name (defaults to "key")
  -- vault_key = "password"
}
request:basic_auth('username', passwd)

local response = request:send()
```
