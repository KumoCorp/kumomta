---
description: "Set per-tenant or per-queue send rate limits — max_message_rate, overall_max_message_rate, and daily caps with additional_message_rate_throttles."
---

# How Do I Set Per-Tenant or Per-IP Send Rate Limits (Hourly and Daily)?

## Per-hour / per-second rate

Use `max_message_rate` on the queue. Note that this applies per queue even when set at a less specific scope, where a queue is `campaign@tenant:domain`:

```toml
[queue.'gmail.com'.'mytenant']
max_message_rate = '100/s'
```

To cap the *combined* flow for a scope, regardless of campaign or destination, use `overall_max_message_rate`:

```toml
[tenant.'mytenant']
overall_max_message_rate = '100/s'
```

## Daily caps

For a daily limit, use `additional_message_rate_throttles` on the egress path, which layers a longer-period throttle alongside the per-second or per-hour one:

```toml
["default"]
max_message_rate = '500/s'
additional_message_rate_throttles = { "daily" = "1000000/day" }
```

When several throttles apply, the effective rate is the smallest of them.

!!! note
    Rate limits for a tenant belong in `queues.toml`, **not** in `shaping.toml`. Shaping is keyed on (source → destination site); the receiving server only sees your IP, not your tenant, so tenant limits cannot be expressed there.

## See also

* [Configuring Queue Management](../userguide/configuration/queuemanagement.md)
* [make_egress_path / additional_message_rate_throttles](../reference/kumo/make_egress_path/additional_message_rate_throttles.md)
