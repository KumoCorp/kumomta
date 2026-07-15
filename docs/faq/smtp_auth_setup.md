---
description: "Set up SMTP AUTH for injection with smtp_server_auth_plain — AUTH PLAIN only, TLS required, and why trusted hosts skip authentication."
---

# How Do I Set Up SMTP AUTH for Injection?

To accept authenticated injection, implement the `smtp_server_auth_plain` event:

```lua
kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta)
  if password == '' then
    return false
  end
  -- look the credential up in a table, database, or Vault
  return password == lookup_password_for(authc)
end)
```

Three things trip people up.

## AUTH PLAIN only

KumoMTA implements AUTH PLAIN. `AUTH LOGIN` is deprecated and is not supported, and there is no Lua hook to add it. If a legacy client can only do `AUTH LOGIN`, front KumoMTA with another MTA (such as Postfix) that accepts `AUTH LOGIN` and relays onward.

## TLS is required for AUTH

Authentication is only offered and accepted over a TLS-protected connection (after STARTTLS). KumoMTA will not accept `AUTH PLAIN` credentials over a cleartext connection, because they are only base64-encoded. If you see no auth attempts at all, confirm the client is negotiating STARTTLS first.

## Trusted hosts skip authentication

The auth handler is not called for peers already permitted by `trusted_hosts` / `relay_hosts`. Do not add the IP of an authenticating client to `relay_hosts` — if you do, it is treated as trusted and never prompted to authenticate.

!!! note
    `require_authz` binds a user to a **tenant name**, not to a sending domain. If you need to enforce that an authenticated user may only send from a particular domain, add that check in Lua at reception.

## See also

* [Checking Inbound SMTP Authentication](../userguide/policy/inbound_auth.md)
* [smtp_server_auth_plain](../reference/events/smtp_server_auth_plain.md)
* [Why is KumoMTA Accepting Connections From Systems Not Listed in relay_hosts?](non_relay_hosts.md)
