---
tags:
 - memory
---

# shrink_policy

{{since('2025.03.19-1d3f1f67')}}

!!! danger
    This is an advanced option whose use is not recommended without
    deep knowledge of KumoMTA and the overall system constraints.

Controls the message shrinking policy when messages are placed into the
corresponding scheduled queue with a due date in the future.

The default behavior when the policy is left unspecified, and for older
versions of KumoMTA, is `"ShrinkDataAndMeta"`, which releases both the message
data and metadata, resulting in a minimal per-message overhead while messages
are scheduled for delivery in the future.

The purpose of `shrink_policy` is to adjust how aggressively that memory
is released depending on how long it will be before the message will
next be retried.

The `shrink_policy` option is an array of `QueueShrinkPolicy` values that have the
following fields:

 * `interval` - a time interval expressed using a duration string like `"60 s"`
   for 60 seconds.  This value will be compared against the time until the
    message is next due for delivery to decide whether the entry matches.
 * `policy` - a `MemoryReductionPolicy` value, which can be one of:
    * `"ShrinkDataAndMeta"` - release both the message data and metadata
    * `"ShrinkData"` - release just the message data, keeping metadata in memory
    * `"NoShrink"` - do not release either data or metadata (very dangerous!
      use with caution!)

The way that `shrink_policy` is processed is that each element of the policy is
compared against the remaining time until due. If the the remaining time is >=
the interval of the entry, then the policy value from that entry is selected.
This repeats until the end of the policy list has been processed.

As an example, if you configure the `shrink_policy` like this:

```lua
kumo.on(
  'get_queue_config',
  function(domain_name, tenant, campaign, routing_domain)
    return kumo.make_queue_config {
      max_retry_interval = '20 minutes',
      shrink_policy = {
        { interval = '0 s', policy = 'ShrinkData' },
        { interval = '60 s', policy = 'ShrinkDataAndMeta' },
      },
    }
  end
)
```

then the system behavior will be:

* When a message is added to the scheduled queue and it is due within the next
  `60s`, then we'll release just the message data and keep the message metadata
  in memory.

* If the message is due further into the future than `60s`, then both the data
  and metadata will be freed from RAM.

It might be advantageous to configure the `shrink_policy` in this way if you
have a lot of available RAM and want to avoid spool IO for messages that will
be retried in the near future, especially if you think that when the message is
next due it might be subject to a suspension or bounce: you can save a load
operation to recall the message metadata to compare against the
suspension/bounce rules in that situation.

!!! caution
    When memory usage hits the soft limit, KumoMTA assumes that messages
    in the scheduled queue are fully "shrunk", having neither their data
    or metadata loaded.

    When you use `shrink_policy` to change that assumption, the effectiveness
    of its low memory recovery options is reduced because it does not sweep
    the scheduled queues to ensure that all the messages are fully shrunk.

    You need to satisfy yourself that you have appropriate constraints
    configured to avoid an OOM-related kill when you change the
    `shrink_policy`.
