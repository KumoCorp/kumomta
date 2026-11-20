# enable_auth_login

{{since('dev')}}

When set to `true`, this listener offers `AUTH LOGIN` alongside `AUTH PLAIN`,
and accepts it from clients.  The default is `false`.

`LOGIN` has never been specified; it is a legacy mechanism that exchanges the
username and password as two separate base64 encoded lines.

Like `AUTH PLAIN`, `AUTH LOGIN` is only offered and accepted once the session
is encrypted; see [tls_mode](tls_mode.md).

Both mechanisms are validated by the same
[smtp_server_auth_plain](../../events/smtp_server_auth_plain.md) event, so no
policy change is needed to accept `LOGIN`.  `LOGIN` has no authorization
identity, so the event is passed the authenticated identity as both *authz* and
*authc*.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:587',
  enable_auth_login = true,
}
```

This option can also be set inside a [peer](peer.md) or [via](via.md) block, to
offer `LOGIN` only to the clients that need it:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:587',
  peer = {
    ['192.168.1.0/24'] = {
      enable_auth_login = true,
    },
  },
}
```
