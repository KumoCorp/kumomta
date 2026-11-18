# enable_tls

Controls whether and how TLS will be used when connecting to the destination.

!!! note
    When [enable_mta_sts](enable_mta_sts.md) and/or [enable_dane](enable_dane.md)
    are enabled and the destination publishes a policy, that policy adjusts
    the effective value of this setting as follows.

|Behavior|Since|
|--------|-----|
|The policy can raise the effective value, such as from `Opportunistic` to `Required`, but never lowers it: it cannot make STARTTLS optional, or skip certificate verification, where your configured value demands them|{{since('dev', inline=True)}}|
|The policy replaces this setting's value. DANE `TLSA` records that are present but unusable lower it to `RequiredInsecure`. An MTA-STS `testing` policy lowers it to `OpportunisticInsecure`, except that it stays `RequiredInsecure` when those unusable DANE records were found|{{since('2026.09.22-a276d4a8', inline=True)}}|
|The policy replaces your configured value. An MTA-STS `testing` policy lowers it to `OpportunisticInsecure`|Releases before 2026.09.22-a276d4a8|

Possible values are:

* `"Opportunistic"` - use TLS if advertised by the `EHLO` response. If the peer
  has invalid or self-signed certificates, then the delivery will fail. KumoMTA
  will NOT fallback to not using TLS on that same host.

* `"OpportunisticInsecure"` - use TLS if advertised by the `EHLO` response.
  Validation of the certificate will be skipped. Not recommended for sending to
  the public internet; this is intended for local or lab testing scenarios.

* `"Required"` - Require that TLS be advertised in the `EHLO` response. The
  remote host must have valid certificates in order to deliver to the site.

* `"RequiredInsecure"` - Require that TLS be advertised in the `EHLO` response.
  Validation of the certificate will be skipped.  Not recommended for sending
  to the public internet; this is intended for local or lab testing scenarios.

* `"Disabled"` - do not try to use TLS.

The default value is `"Opportunistic"`.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
  }
end)
```


