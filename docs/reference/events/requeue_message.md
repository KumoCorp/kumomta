# `kumo.on('requeue_message', function(message, smtp_response))`

{{since('2024.11.08-d383b033')}}

!!! note
    This event was actually added in `2024.06.10-84e84b89` but under
    the erroneous name `message_requeued`, and with a broken event
    registration that prevented it from working. That was corrected
    in the version shown above when the `smtp_response` parameter was added.

This event is triggered when a message encountered a transient failure.
Its purpose is to allow you to re-bind the message to an alternative
queue, perhaps to relay it via an alternate tier or to use an alternative
pool for egress.

The `smtp_response` parameter is a one-line rendition of the SMTP
response that resulted in the message being requeued. There are a couple
of internal triggers for a requeue that are not directly caused by
an SMTP response. Those responses have `KumoMTA internal:` prefixed
the to textual portion of the response.

Multiple instances of the `requeue_message` event can be registered,
and they will be called in the order in which they were registered,
until all registered events are called, or until one explicitly
returns `nil` to signal that no more should be triggered.

The event is triggered prior to incrementing the number of attempts,
so `message:num_attempts()` will return one less than the current
number.

In order to re-bind the message you will typically alter one or more of the
meta values of the message that impact the queue name:

* `queue`
* `routing_domain`
* `tenant`
* `campaign`

See [Queues](../queues.md) for more information.

If the effective queue name for the message is changed as a result of
dispatching the `requeue_message` event, then the message will be immediately
eligible for delivery in the context of its new queue, however, if the message
has scheduling constraints set via
[msg:set_scheduling](../message/set_scheduling.md) those will remain in effect
unless you explicitly clear them.  The reason for this is that kumod doesn't
have any implicit knowledge of the semantics of the queue, so it doesn't know
whether the scheduling constraints should remain in force or not.

In the example below, a message is re-routed to a smart host after
the third attempt to send it encounters a transient failure.

```lua
local SMART_HOST = '[10.0.0.1]'

kumo.on('requeue_message', function(msg)
  local queue = msg:queue_name()
  if queue ~= SMART_HOST and msg:num_attempts() >= 2 then
    -- Re-route to alternative infra to manage the rest of the send
    msg:set_meta('queue', SMART_HOST)
    -- clear any scheduling constraints, as they do not apply
    -- when sending via a smart host
    msg:set_scheduling(nil)
  end
end)
```

Calling [kumo.reject](../kumo/reject.md) to raise an error in your event
handler (regardless of the code parameter passed to `kumo.reject`) will
cause the message to bounced; a `Bounce` record will be logged and the
message will be removed from the spool.

Any other kind of error raised by the event handler will cause the error
to be logged to the diagnostic log, and the message returned to its
original scheduled queue.

