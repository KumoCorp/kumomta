# tls_mode

{{since('dev')}}

Selects how TLS is established when connecting to the destination. This is a
separate axis from [enable_tls](enable_tls.md): `enable_tls` expresses the
security posture (whether TLS is attempted or required, and whether the peer
certificate is validated), while `tls_mode` selects the transport.

Possible values are:

* `"StartTls"` - connect in cleartext and upgrade to TLS via the `STARTTLS`
  command, as governed by [enable_tls](enable_tls.md). This is the default.

* `"ImplicitTls"` - the connection is TLS from the first byte (often referred to
  as SMTPS). The handshake is performed immediately after the TCP connection is
  established, before the banner is read. The `STARTTLS` command is not used.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    tls_mode = 'ImplicitTls',
  }
end)
```

In `ImplicitTls` mode the TLS handshake always happens, and a failed handshake
is a hard connection failure.  The `enable_tls` option is consulted only to
decide whether insecure certificate validation should be used.

* an insecure variant (`OpportunisticInsecure` or `RequiredInsecure`) skips
  validation, giving unauthenticated implicit TLS.  This is not recommended
  for production use.

* any other variant (including `Disabled`) requires a valid certificate.

If you must connect with insecure implicit TLS, you should combine
`tls_mode = 'ImplicitTls'` with `enable_tls = 'OpportunisticInsecure'`.

[enable_dane](enable_dane.md) and [enable_mta_sts](enable_mta_sts.md) are
ignored when `tls_mode` is `"ImplicitTls"`. Both DANE and MTA-STS are defined
only for the `STARTTLS`-on-port-25 relay path, and have nothing to say about
an implicit TLS connection.

Set [smtp_port](smtp_port.md) to the port the destination serves implicit TLS
on (commonly `465`).
