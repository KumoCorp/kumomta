# enable_tls

Controls whether and how TLS will be used when connecting to the destination.

!!! note
    This setting is overriden by [enable_mta_sts](enable_mta_sts.md) and/or
    [enable_dane](enable_dane.md) when either of those options are enabled and
    an MTA-STS or DANE policy (respectively) is published by the destination
    site.

Possible values are:

* `"Opportunistic"` - use TLS if advertised by the `EHLO` response. If the peer
  has invalid or self-signed certificates, then the delivery will fail. KumoMTA
  will NOT fallback to not using TLS on that same host.

* `"OpportunisticInsecure"` - use TLS if advertised by the `EHLO` response.
  Validation of the certificate will be skipped. If the handshake fails, retry
  the same address once in plaintext on a fresh connection within the current
  delivery attempt. Not recommended for sending to the public internet; this
  is intended for local or lab testing scenarios.

* `"Required"` - Require that TLS be advertised in the `EHLO` response. The
  remote host must have valid certificates in order to deliver to the site.

* `"RequiredInsecure"` - Require that TLS be advertised in the `EHLO` response.
  Validation of the certificate will be skipped.  Not recommended for sending
  to the public internet; this is intended for local or lab testing scenarios.

* `"Disabled"` - do not try to use TLS.

The default value is `"Opportunistic"`.

{{since('dev', inline=True)}}
A failed TLS handshake closes the connection. SMTP does not resume in plaintext
on that socket. `OpportunisticInsecure` retries the same address once on a fresh
plaintext connection even when
[opportunistic_tls_reconnect_on_failed_handshake](opportunistic_tls_reconnect_on_failed_handshake.md)
is `false`. That retry does not by itself remember the site as having broken TLS
or make other candidates skip STARTTLS. If establishing that connection fails,
delivery proceeds to the remaining addresses in the connection plan.

When the reconnect option is disabled, a `530` reply mentioning STARTTLS at
MAIL FROM on this implicit plaintext retry causes KumoMTA to close the connection
and try the next address in the connection plan, rather than bounce the message.
If there are no remaining addresses, the message defers. This does not retry the
refused address again, even with `reconnect_strategy = "ReconnectSameHost"`.
Other SMTP transaction rejections retain their normal defer or bounce handling.

`Opportunistic` requires the reconnect option for immediate plaintext retry.
[remember_broken_tls](remember_broken_tls.md) can make subsequent opportunistic
connections skip STARTTLS. Neither mechanism permits plaintext when the effective
policy requires TLS. STARTTLS command rejection, handshake timeout, and TLS setup
errors are not automatic plaintext fallback triggers. Failure of EHLO after a
successful handshake requires the reconnect option for a plaintext retry in either
opportunistic mode.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
  }
end)
```


