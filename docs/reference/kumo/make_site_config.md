# `kumo.make_site_config { PARAMS }`

Constructs a configuration object that specifies how a *site* will behave.

This function should be called from the
[get_site_config](../events/get_site_config.md) event handler to provide the
configuration for the requested site.

The following keys are possible:

## connection_limit

Specifies the maximum number of concurrent connections that will be made from
the current MTA machine to the destination site.

```lua
kumo.on('get_site_config', function(domain, site_name)
  return kumo.make_site_config {
    connection_limit = 32,
  }
end)
```

## enable_tls

Controls whether and how TLS will be used when connecting to the destination.
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

The default value is `"Opportunistic"`.

```lua
kumo.on('get_site_config', function(domain, site_name)
  return kumo.make_site_config {
    enable_tls = 'Opportunistic',
  }
end)
```

## idle_timeout

Controls how long a connection will remain open and idle, waiting to be
reused for another delivery attempt, before being closed.

The value is specified in seconds.

```lua
kumo.on('get_site_config', function(domain, site_name)
  return kumo.make_site_config {
    idle_timeout = 60,
  }
end)
```

## max_ready

Specifies the maximum number of messages that can be in the *ready queue*.
The ready queue is the set of messages that are immediately eligible for delivery.

If a message is promoted from its delayed queue to the ready queue and it would
take the size of the ready queue above *max_ready*, the message will be delayed
by a randomized interval of up to 60 seconds before being considered again.


