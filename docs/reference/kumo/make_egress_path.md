# `kumo.make_egress_path { PARAMS }`

Constructs a configuration object that specifies how traffic travelling the
path from a *source* to a *site* will behave.

This function should be called from the
[get_egress_path_config](../events/get_egress_path_config.md) event handler to provide the
configuration for the requested site.

The following keys are possible:

## connection_limit

Specifies the maximum number of concurrent connections that will be made from
the current MTA machine to the destination site.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = 32,
  }
end)
```

## consecutive_connection_failures_before_delay

Each time KumoMTA exhausts the full list of hosts for the destination it
increments a `consecutive_connection_failures` counter. When that counter
exceeds the `consecutive_connection_failures_before_delay` configuration value,
KumoMTA will then delay all of the messages currently in the ready queue,
generating a transient failure log record with code `451 4.4.1 No answer from
any hosts listed in MX`.

The default value for this setting is 100.

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
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
  }
end)
```

## idle_timeout

Controls how long a connection will remain open and idle, waiting to be
reused for another delivery attempt, before being closed.

The value is specified in seconds.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    idle_timeout = 60,
  }
end)
```

## max_connection_rate

Optional string.

Specifies the maximum permitted rate at which connections can be established
from this source to the corresponding destination site.

The value is of the form `quantity/period`
where quantity is a number and period can be a measure of time.

Examples of throttles:

```
"10/s" -- 10 per second
"10/sec" -- 10 per second
"10/second" -- 10 per second

"50/m" -- 50 per minute
"50/min" -- 50 per minute
"50/minute" -- 50 per minute

"1,000/hr" -- 1000 per hour
"1_000/h" -- 1000 per hour
"1000/hour" -- 1000 per hour

"10_000/d" -- 10,000 per day
"10,000/day" -- 10,000 per day
```

Throttles are implemented using a Generic Cell Rate Algorithm.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    max_connection_rate = '100/min',
  }
end)
```

If the throttle is exceeded and the delay before a connection be established
is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

## max_deliveries_per_connection

Optional number.

If set, no more than this number of messages will be attempted on any
given connection.

If unset, there is no limit.

## max_message_rate

Optional string.

Specifies the maximum permitted rate at which messages can be delivered
from this source to the corresponding destination site.

The throttle is specified the same was as for `max_connection_rate` above.

If the throttle is exceeded and the delay before the current message can be
sent is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

## max_ready

Specifies the maximum number of messages that can be in the *ready queue*.
The ready queue is the set of messages that are immediately eligible for delivery.

If a message is promoted from its delayed queue to the ready queue and it would
take the size of the ready queue above *max_ready*, the message will be delayed
by a randomized interval of up to 60 seconds before being considered again.

## smtp_port

Specified the port to connect to when making an SMTP connection to a destination
MX host.

The default is port 25.

See also [kumo.define_egress_source().remote_port](define_egress_source.md#remote_port)
