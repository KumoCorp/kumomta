---
tags:
  - xfer
---
# kumo.xfer.xfer

{{since('dev')}}

```lua
kumo.xfer.xfer(msg, target_url, opt_reason)
```

Adjusts the queue parameters on `msg` (which must be a
[Message](../message/index.md) object) such that it will be transferred to the
kumomta node identified by the `target_url` parameter.

The `target_url` is the URL prefix of the HTTP Listener for the destination
kumomta node, including any port number, but excluding any path or query
components.  For example `http://10.0.0.2:8000` is how a valid prefix might
appear, but it need not be using an IP address, any valid URL prefix of that
form is acceptable, provided that it is running a compatible version of
KumoMTA.

If `opt_reason` is specified, it will be used as the reason string in an
`AdminRebind` log record to capture any queue change that might occur due to
calling this function.

If `opt_reason` is not specified (omitted, or is explicitly set to `nil`),
then no `AdminRebind` log entry will be produced.

If the message is already configured to xfer to `target_url` then no changes
will be made to the message, and no log record will be logged by this
particular call.

If the message is configured to xfer to a different target, then that xfer
will be cancelled and then the message will be configured to xfer to
`target_url`.

!!! note
    This function does not alter any of the scheduling parameters on the
    message.  It is NOT intended to be used as part of retry processing.  You
    should look at [kumo.xfer.xfer_in_requeue](xfer_in_requeue.md) for a
    function that does handle that situation.

