---
description: "Why KumoMTA memory usage climbs — resident messages in the Ready Queues, sizing max_ready, and the metrics that show what is using memory."
---

# Why Is KumoMTA Using So Much Memory?

KumoMTA keeps message bodies in RAM on purpose; a large percentage of messages deliver on the first attempt, and if a message can be retained in memory between injection and delivery KumoMTA saves a lot of I/O. Memory usage therefore scales with how many messages are waiting to be delivered. When egress slows or stops, messages accumulate and memory climbs with them.

The dominant contributor is resident messages in the Ready Queues: a message's body stays in memory until after its first delivery attempt. As a rule of thumb:

```txt
RAM per ready queue ≈ max_ready × average message size
```

A queue with `max_ready = 1000` and an average message size of 100 KB budgets roughly 100 MB in the worst case, for that single queue. Set `max_ready` very high, let egress to a destination stall (a throttling provider, or every source suspended), and that queue fills to its limit and sits on the memory.

## How to control it

Keep `max_ready` at a modest default in your shaping configuration and raise it only for your top few destinations by volume:

```toml
# shaping.toml
["default"]
max_ready = 1000

["gmail.com"]
max_ready = 10000
```

Size `max_ready` as a small multiple of your sustained per-second egress rate for that destination (around 2000 for a queue that sustains 1000/s). Over-provisioning is what turns a temporary egress problem into a memory problem.

!!!! note
    `max_ready` is configured as a __per queue__ value, where each queue represents a potential connection from a given egress source to a given site name. Always keep your overall connection count in mind when configuring `max_ready`.

## How to see what is using memory

* `kcli top` shows live memory statistics and per-context usage.
* The Prometheus metrics endpoint exposes `memory_usage`, `memory_limit`, `message_count`, and `message_data_resident_count`. A node holding hundreds of thousands of resident messages is almost always blocked from delivering.

When the system reaches its memory limit it sheds load automatically, shrinking message bodies to spool and refusing new injections until it recovers. That is a safety net, not a fix. If messages genuinely cannot move, address the underlying delivery problem.

!!! note
    `VmRSS` is the real footprint to watch; `VmSize` (virtual size) can look alarmingly large but is not the resident usage.

If memory is climbing because mail is not leaving the server, see [What Do "ReadyQueueWasFull" / "DueTimeWasReached" Messages Mean?](ready_queue_was_full.md) and [Why Are All Sources Suspended?](all_sources_suspended.md).

## See also

* [Memory Management](../reference/memory.md)
* [make_egress_path / max_ready](../reference/kumo/make_egress_path/max_ready.md)
* [Understanding Message Flows](../userguide/performance/messageflow.md)
