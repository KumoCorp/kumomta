---
description: "What ReadyQueueWasFull and DueTimeWasReached mean in KumoMTA logs — why the Ready Queue cannot drain and how to triage it."
---

# What Do "ReadyQueueWasFull" and "DueTimeWasReached" Mean?

These messages appear when mail is being accepted but not leaving the server. Neither is an error. Each one describes a message that was returned to the Scheduled Queue to wait, rather than delivered.

When KumoMTA moves a message from the Scheduled Queue to the Ready Queue, it first checks whether the Ready Queue has room (governed by `max_ready`). If it is full, the message is delayed by a small random interval, returned to the Scheduled Queue, and a `Delayed` log record is written with a reason such as `ReadyQueueWasFull`. `DueTimeWasReached` means the message's next-attempt time arrived and it is being evaluated again.

A steady stream of these records means the Ready Queue cannot drain. Mail is arriving faster than it can egress.

## The three usual causes

1. **All sources for the pool are suspended.** Spam complaints or block responses trigger Traffic Shaping Automation to suspend sources, so there is nothing to send from. See [Why Are All Sources Suspended?](all_sources_suspended.md)
2. **The destination is throttling you.** The mailbox provider is limiting connections or rate based on your reputation, so the Ready Queue drains slowly. Review your shaping for that destination.
3. **`max_ready` is too small** for the destination's sustained rate, so messages oscillate between the Scheduled and Ready Queues. See [Why Is KumoMTA Using So Much Memory?](high_memory_usage.md)

## How to triage

```console
# Where is volume sitting, and which destinations are limited?
$ kcli queue-summary --by-volume

# Is anything suspended?
$ kcli top
```

Then check the metrics for the affected queues: `delayed_due_to_ready_queue_full`, `delayed_due_to_message_rate_throttle`, and `ready_full` tell you whether you are looking at a full Ready Queue, a throttle, or a suspension. Read the transient-failure log records for the destination to see the provider's actual responses.

## See also

* [Troubleshooting KumoMTA](../userguide/operation/troubleshooting.md)
* [Understanding Message Flows](../userguide/performance/messageflow.md)
* [kumod Metrics](../reference/metrics/kumod/index.md)
