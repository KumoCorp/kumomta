---
description: "What 'no sources are eligible for selection' means — how TSA suspensions work, how to inspect them with kcli, and what to do about them."
---

# Why Are All Sources Suspended ("No Sources Are Eligible For Selection")?

A log message such as `no sources for pool='...' are eligible for selection`, or `all possible sources for <domain> are suspended`, means every egress source in the pool that would be used for a destination has been suspended (or the pool is empty). With nothing to send from, messages for that destination accumulate in the queues.

While this can be caused by not having any properly configured and connected egress sources (a configuration or networking issue), the most common cause is Traffic Shaping Automation (TSA): a rule reacted to a block or complaint response from the mailbox provider (a Yahoo `[TS04]` deferral, for example) and suspended the source for a defined duration.

!!! note
    Suspensions are scoped to the (egress source × destination site) combination. A source suspended for Gmail still sends to Yahoo, so you do not need separate pools per provider. KumoMTA uses a non-suspended source wherever one is available.

## How to inspect suspensions

```console
$ kcli suspend-list
$ kcli suspend-ready-q-list
```

These show what is currently suspended and why. You can also query the admin API at `/api/admin/suspend/v1` and `/api/admin/suspend-ready-q/v1`.

## What to do about it

Read the transient-failure log records to find the response that triggered the suspension. If the suspension is more aggressive than your situation calls for, shorten the duration in your TSA rule, or have TSA bounce the affected messages instead of letting them pile up. And treat repeated suspensions as what they are — a reputation signal. The lasting fix is resolving the underlying complaint or block problem with the provider.

If suspended sources are causing memory to climb, see [Why Is KumoMTA Using So Much Memory?](high_memory_usage.md) and [What Do "ReadyQueueWasFull" / "DueTimeWasReached" Mean?](ready_queue_was_full.md)

## See also

* [Traffic Shaping Automation](../userguide/trafficshaping/automation.md)
* [make_egress_path](../reference/kumo/make_egress_path/index.md)
