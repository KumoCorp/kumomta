# requeue_message

```lua
kumo.on(
  'requeue_message',
  function(
    message,
    smtp_response,
    insert_context,
    increment_attempts,
    delay
  )
  end
)
```

{{since('2024.11.08-d383b033')}}

!!! note
    This event was actually added in `2024.06.10-84e84b89` but under
    the erroneous name `message_requeued`, and with a broken event
    registration that prevented it from working. That was corrected
    in the version shown above when the `smtp_response` parameter was added.

This event is triggered when a message encountered a transient failure
and will be re-inserted into an appropriate scheduled queue.

Its purpose is to allow you to re-bind the message to an alternative
queue, perhaps to relay it via an alternate tier or to use an alternative
pool for egress.

This event has evolved to include more context over time.  The
meaning of each parameter, along with the version in which it was
introduced, is shown below:

   * `message` - is the [Message](../message/index.md) object which is being re-queued.
   * `smtp_response` is a one-line rendition of the SMTP response that resulted
     in the message being requeued. There are a couple of internal triggers for
     a requeue that are not directly caused by an SMTP response. Those
     responses have `KumoMTA internal:` prefixed the to textual portion of the
     response.
   * `insert_context` {{since('dev', inline=True)}} is an array holding the
     reason(s) why the message is being inserted into the queue manager.  There
     will typically be 1 reason, but it is possible to have multiple reasons to
     indicate eg: that we just received or loaded a message from spool and then
     encountered a transient failure.  Each element of the array is a string.
     Possible reasons include:

       * `"Received"` - Message was just received.
       * `"Enumerated"` - Message was just loaded from spool
       * `"ScheduledForLater"` - Message had its due time explicitly set.
       * `"ReadyQueueWasSuspended"`
       * `"MessageRateThrottle"`
       * `"ThrottledByThrottleInsertReadyQueue"`
       * `"ReadyQueueWasFull"`
       * `"FailedToInsertIntoReadyQueue"`
       * `"MessageGetQueueNameFailed"`
       * `"AdminRebind"`
       * `"DueTimeWasReached"`
       * `"MaxReadyWasReducedByConfigUpdate"`
       * `"ReadyQueueWasDelayedDueToLowMemory"`
       * `"FailedDueToNullMx"`
       * `"MxResolvedToZeroHosts"`
       * `"MxWasProhibited"`
       * `"MxWasSkipped"`
       * `"TooManyConnectionFailures"`
       * `"TooManyRecipients"`
       * `"ConnectionRateThrottle"`
       * `"LoggedTransientFailure"` - There was a TransientFailure logged to
         explain what really happened.  The information contained in the reason
         may not represent the full extent of the situation.

   * `increment_attempts` {{since('dev', inline=True)}} - a boolean value
     that will be set to `true` if the number of attempts on the message
     would be incremented as part of normal processing of the requeue
     event.  Not every requeue situation will increment this counter.
   * `delay` {{since('dev', inline=True)}} a [TimeDelta](../kumo.time/TimeDelta.md)
     object indicating a suggested delay to be applied to the message.
     This will typically be `nil` which indicates that the usual retry
     parameters for the associated queue should be used, but in some
     cases (eg: throttling) it may be set to a duration indicating
     when the throttle may open back up.

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

kumo.on(
  'requeue_message',
  function(msg, smtp_response, insert_context, increment_attempts, delay)
    local queue = msg:queue_name()
    if queue ~= SMART_HOST and msg:num_attempts() >= 2 then
      -- Re-route to alternative infra to manage the rest of the send
      msg:set_meta('queue', SMART_HOST)
      -- clear any scheduling constraints, as they do not apply
      -- when sending via a smart host
      msg:set_scheduling(nil)
    end
  end
)
```

Calling [kumo.reject](../kumo/reject.md) to raise an error in your event
handler (regardless of the code parameter passed to `kumo.reject`) will
cause the message to bounced; a `Bounce` record will be logged and the
message will be removed from the spool.

Any other kind of error raised by the event handler will cause the error
to be logged to the diagnostic log, and the message returned to its
original scheduled queue.

