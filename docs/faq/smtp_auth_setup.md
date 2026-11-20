---
description: "Set up SMTP AUTH for injection with smtp_server_auth_plain — PLAIN by default, LOGIN on request, TLS required, and why trusted hosts skip authentication."
---

# How Do I Set Up SMTP AUTH for Injection?

To accept authenticated injection, implement the `smtp_server_auth_plain` event:

```lua
-- Use this to lookup and confirm a user/password credential
kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta)
  -- This is just an example of how to populate the return value,
  -- not a recommended way to handle passwords in production!
  -- In particular, it is an absolutely terrible idea to hard code
  -- a password here in plain text!
  local password_database = {
    ['daniel'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[authc] == password
end)
```

Three things to watch for:

## AUTH PLAIN by default, AUTH LOGIN on request

KumoMTA offers `AUTH PLAIN`. For legacy clients that can only do `AUTH LOGIN`, set [enable_auth_login](../reference/kumo/start_esmtp_listener/enable_auth_login.md) on the listener to offer that mechanism as well. The same `smtp_server_auth_plain` event validates both, so the handler above needs no changes; `LOGIN` has no authorization identity, so it is called with *authz* set to the same value as *authc*.

{{since('dev', indent=True)}}
    `AUTH LOGIN` support is new as of this release. Prior to it, fronting KumoMTA with another MTA that accepts `AUTH LOGIN` was the only option.

## TLS is required for AUTH

Authentication is only offered and accepted over a TLS-protected connection (after STARTTLS). KumoMTA will not accept `AUTH PLAIN` credentials over a cleartext connection, because they are only base64-encoded. If you see no auth attempts at all, confirm the client is negotiating STARTTLS first.

## Trusted hosts skip authentication

The auth handler is not called for peers already permitted by `trusted_hosts` and `relay_hosts`. Do not add the IP of an authenticating client to `relay_hosts` — if you do, it is treated as trusted and never prompted to authenticate.

!!! note
    `require_authz` binds a user to a **tenant name**, not to a sending domain. If you need to enforce that an authenticated user may only send from a particular domain, add that check in Lua at reception.

## See also

* [Checking Inbound SMTP Authentication](../userguide/policy/inbound_auth.md)
* [smtp_server_auth_plain](../reference/events/smtp_server_auth_plain.md)
* [Why is KumoMTA Accepting Connections From Systems Not Listed in relay_hosts?](non_relay_hosts.md)
