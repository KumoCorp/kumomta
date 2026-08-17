# dispatcher_progress_watchdog_timeout

{{since('dev')}}

Sets the maximum duration a connection-handling dispatcher task for
this egress path may go without making any forward progress before the
maintainer aborts it.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    dispatcher_progress_watchdog_timeout = '5m',
  }
end)
```

The value is a humantime duration string (e.g. `'30s'`, `'2m'`,
`'10m'`), or `nil` (the default) to use the value derived at runtime
from the egress path's other timeouts and the scheduled queue's
protocol. For SMTP-family protocols the derived value is
`max(2 * longest per-command timeout, 60s)`; for protocols that
delegate to user code or external services and do not expose a
per-operation timeout, it is 10 minutes.

If you set a `max_batch_latency` larger than the derived default, set
this option explicitly so the watchdog does not flag batch
accumulation as a wedge.

When the watchdog fires, the connection's in-flight message is returned
to the scheduled queue for another delivery attempt, and a `Delayed`
record is written to the delivery log identifying the watchdog as the
cause:

```json
{
  "type": "Delayed",
  "recipient": "victim@wedge.example.com",
  "queue": "wedge.example.com",
  "response": {
    "code": 451,
    "enhanced_code": {"class": 4, "subject": 4, "detail": 1},
    "content": "dispatcher watchdog aborted task; phase=DeliveringMessage detail=\"lua: send\" session=db5e6704-05d3-4c8b-9cac-e3edd39e5c7a age=2s 481ms 890us 590ns time_in_phase=2s 478ms 972us 45ns"
  },
  "session_id": "db5e6704-05d3-4c8b-9cac-e3edd39e5c7a"
}
```

The `session` value embedded in the response, and the `session_id`
field on the log record, both match the id present on the diagnostic
that kumod logs at the time of the abort, so the two can be correlated.

The
[dispatcher_watchdog_aborted_total](../../metrics/kumod/dispatcher_watchdog_aborted_total.md)
prometheus counter increments each time the watchdog fires.
