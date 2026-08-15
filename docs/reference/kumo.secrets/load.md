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
  -- Optional: bound the request (defaults to "30 seconds")
  -- vault_timeout = "10 seconds"
}
request:basic_auth('username', passwd)

local response = request:send()
```

## Bounding a vault read

{{since('dev')}}

Vault reads are bounded by the [keysource](../keysource.md)'s `vault_timeout`,
which defaults to `30 seconds`. When the timeout is hit, `kumo.secrets.load`
raises an error, which you can catch with `pcall`.

Bounding this matters more than it might appear. `kumo.secrets.load` is
often called from a connection handler such as
[smtp_server_auth_plain](../events/smtp_server_auth_plain.md), and an SMTP
session holds an activity token for its whole lifetime. A vault endpoint
that accepts the TCP connection but never answers would, before this
option existed, park that session forever — and a graceful shutdown waits
for every session to finish, so one stuck read can hold the whole process
open until it is killed.

Note that
[data_processing_timeout](../kumo/start_esmtp_listener/data_processing_timeout.md)
does not cover this: it bounds the post-`DATA` callbacks only, not
callbacks such as `smtp_server_auth_plain` that run earlier in the session.
