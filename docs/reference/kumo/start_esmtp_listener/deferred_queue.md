# deferred_queue

{{since('dev')}}

!!! warning
    Carefully read and understand this option before enabling it.
    It should *NOT* be used on the public internet, and only enabled
    for listeners where you implicitly trust any incoming connection.

When this option is set to `true`, after receiving the DATA portion of an
incoming message, the processing flow is altered such that the latency of
normal post-DATA processing is hidden from the injecting client.

!!! info
    Enabling this option will increase your average incoming SMTP transaction
    latency slightly, and increase IOPS pressure to your spool and logging
    devices, but will clamp the worst-case latency outliers for your incoming
    SMTP transaction latency.

* The `smtp_server_message_received` event is *NOT* triggered
* Trace and supplemental headers are produced and added to the message
  according to that unprocessed state of the message
* The message is saved to spool (unless [deferred_spool](deferred_spool.md) is enabled)
* The message is queued to a special queue named `deferred_smtp_inject.kumomta.internal`.
* A `Reception` record is recorded that shows that it was queued to that queue
* A `250 ok` response is returned to the injector.  It is not possible to
  return anything other than a 250 response when `deferred_queue = true`.

The `deferred_smtp_inject.kumomta.internal` queue will process messages
according to these steps:

* The [smtp_server_message_deferred_inject](../../events/smtp_server_message_deferred_inject.md) event will be triggered
* If returns without error:
  * A `Delivery` record is logged for the message showing that the message
    was "delivered" via the `DeferredSmtpInjection` protocol.
  * The message will be rebound into the appropriate queue per the
    normal rules based on the message metadata and envelope recipient.
* If the event triggered a `kumo.reject` with a 5xx code, a `Bounce` record
  will be recorded and the message will be removed from the spool.
* In other error cases, a `TransientFailure` record will be recorded and
  the message will attempt processing again later. The
  `deferred_smtp_inject.kumomta.internal` scheduled queue has a hard-coded base
  retry interval of one minute, and will respect the product default
  exponential backoff for up to 1 week.

```lua
kumo.start_esmtp_listener {
  -- ..
  deferred_queue = true,
}
```

## Shaping Considerations

If you are using the shaping helper, you should consider adding:

```toml
["deferred_smtp_inject.kumomta.internal"]
# The domain is fake and doesn't resolve, don't try to resolve it
mx_rollup = false
# Set connection_limit to a small multiple of the number of
# CPUs in the system.  Experiment to find what works best in
# your environment.  Try either 1x or 2x number of cpus to start.
connection_limit = 12
# Increase max_ready to something appropriate; since this queue
# is a fan-in, it will typically hold more messages than your
# other queues
max_ready = 80000
```

If you are doing shaping via lua, then you can more directly handle
the configuration:

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    if routing_domain == 'deferred_smtp_inject.kumomta.internal' then
      return kumo.make_egress_path {
        connection_limit = kumo.available_parallelism(),
        refresh_strategy = 'Epoch',
        max_ready = 80000,
      }
    end
    -- do the rest of your shaping configuration here
  end
)
```


