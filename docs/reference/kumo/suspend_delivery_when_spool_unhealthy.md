# kumo.suspend_delivery_when_spool_unhealthy

```lua
kumo.suspend_delivery_when_spool_unhealthy(ENABLED)
```

{{since('dev')}}

Controls whether KumoMTA pauses delivery while the spool is in an
unhealthy state, as reflected by the
[rocks_spool_load_shed_active](../metrics/kumod/rocks_spool_load_shed_active.md)
metric.

When `true` (the default), an unhealthy spool causes the delivery pipeline to
treat affected messages as if every source were administratively suspended:
rather than attempting to load and deliver them, the messages are held in their
scheduled queues and retried on their normal next-due schedule. No new delays
are introduced beyond the existing retry timing, so an issue while holding
millions of messages in the spool does not schedule a synchronized retry
stampede.

When the unhealthy condition has cleared (either through operator
intervention or, where the underlying issue was transient, via the
spool's own recovery logic), the held messages resume delivery via
the normal queue maintenance cycle without any further action.

This function should be called only from inside your
[init](../events/init.md) event handler.

Held messages produce a `Delayed` [log record](../log_record.md)
carrying a 451 response and a content string that names the
unhealthy condition. For example:

```json
{
    "type": "Delayed",
    "response": {
        "code": 451,
        "enhanced_code": { "class": 4, "subject": 4, "detail": 4 },
        "content": "KumoMTA internal: delivery suspended: spool unhealthy: the spool is not accepting writes"
    }
}
```

When the unhealthy condition is first encountered later in the
delivery pipeline -- for example, when the dispatcher tries to
load the message body or metadata out of the spool and the read
fails -- the affected message is instead logged as a
`KumoMTA internal` `TransientFailure`. In this scenario the SMTP
transaction for the message had not yet been started:

```json
{
    "type": "TransientFailure",
    "response": {
        "code": 400,
        "content": "KumoMTA internal: error in deliver_message: IO error: No such file or directory: ..."
    }
}
```

!!! danger
    With `kumo.suspend_delivery_when_spool_unhealthy(false)`, delivery
    continues even while the spool is unhealthy. Ingress remains
    refused (the SMTP banner returns 421 and HTTP ingress endpoints
    return 503), but in-flight delivery attempts proceed. **A
    successful SMTP transaction followed by a failed spool `remove()`
    will leave the message in the spool, where it will be retried on
    the next attempt -- producing a duplicate at the recipient.**
    Only disable this if you have an alternative safeguard against
    that class of duplicate.
