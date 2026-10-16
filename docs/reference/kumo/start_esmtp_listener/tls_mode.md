# tls_mode

{{since('dev')}}

Selects how TLS is established on connections accepted by this listener.

Possible values are:

* `"StartTls"` - the connection begins in cleartext and the client upgrades to
  TLS by issuing the `STARTTLS` command. This is the default.

* `"ImplicitTls"` - the connection is TLS from the first byte (often referred to
  as SMTPS). There is no `STARTTLS` negotiation. The listener performs the TLS
  handshake before sending the banner.

```lua
kumo.start_esmtp_listener {
  listen = '0:465',
  tls_mode = 'ImplicitTls',
}
```

TLS uses the same certificate configuration as `STARTTLS`. See
[tls_certificate](tls_certificate.md) and [tls_private_key](tls_private_key.md).
When neither is set, a self-signed certificate is generated for the listener's
[hostname](hostname.md).

When [require_proxy_protocol](require_proxy_protocol.md) is also enabled, the
PROXY protocol header is read in cleartext before the TLS handshake.

Messages received over an `ImplicitTls` connection contain a `tls_implicit`
entry in the connection metadata, and the `Received` header records the session
as `ESMTPS` (or `ESMTPSA` when authenticated), the same as a session that
upgraded via `STARTTLS`.
