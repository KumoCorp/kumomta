---
tags:
  - xfer
---
# kumo.xfer.xfer_in_requeue

{{since('dev')}}

```lua
kumo.xfer.xfer_in_requeue(
  msg,
  target_url,
  insert_context,
  increment_attempts,
  delay,
  opt_reason
)
```

!!! caution
    This function is intended to be called from within the
    [requeue_message](../events/requeue_message.md) event only.  Calling it
    from other contexts may result in non-deterministic and unreliable
    behavior.

Adjusts the queue parameters on `msg` (which must be a
[Message](../message/index.md) object) such that it will be transferred to the
kumomta node identified by the `target_url` parameter.

The `target_url` is the URL prefix of the HTTP Listener for the destination
kumomta node, including any port number, but excluding any path or query
components.  For example `http://10.0.0.2:8000` is how a valid prefix might
appear, but it need not be using an IP address, any valid URL prefix of that
form is acceptable, provided that it is running a compatible version of
KumoMTA.

The `increment_attempts`, `insert_context` and `delay` parameters *MUST* all be
passed through from the parameters with the same names in the
[requeue_message](../events/requeue_message.md) event handler.  They will be
used to decide how to update the scheduling on the message prior to
encapsulating that state into the XFER message framing used to communicate with
the target node.

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
`target_url`.  However, if the message was pending xfer to the alternate
target for a significant amount of time, then the revised schedule computed
for the message may be inaccurate.

## Example: move message to backup tier if underliverable after two attempts

```lua
local BACKUP_HOST = 'http://backup-kumomta:8000'

kumo.on(
  'requeue_message',
  function(msg, smtp_response, insert_context, increment_attempts, delay)
    if msg:num_attempts() >= 2 then
      -- Re-route to alternative infra to manage the rest of the send
      kumo.xfer.xfer_in_requeue(
        msg,
        BACKUP_HOST,
        insert_context,
        increment_attempts,
        delay,
        'reroute to backup infra'
      )
    end
  end
)
```

