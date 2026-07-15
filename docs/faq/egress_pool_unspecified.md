---
description: "Why mail leaves from the wrong IP with egress_pool 'unspecified' — queue helper not applied, or a tenant pointing at an undefined pool."
---

# Why Is My Mail Sending From the Wrong IP? (egress_pool "unspecified")

If your logs show `egress_pool: unspecified` / `egress_source: unspecified`, or mail is leaving from the host's default IP instead of an address in one of your pools, it almost always means no pool was selected for the message, so KumoMTA fell back to the host default.

There is no "IP rotation" setting in KumoMTA. You assign a message's tenant to an egress pool, and the weighted sources within that pool distribute the sends. To pin a single IP, create a pool that contains exactly one source.

## The two usual causes

**1. The queue helper was never applied (or no pool was set).**

Make sure your reception events actually call the queue helper:

```lua
kumo.on('smtp_server_message_received', function(msg)
  queue_helper:apply(msg) -- selects tenant/pool for the message
  dkim_signer(msg) -- signing must come last
end)
```

If you use a custom `get_queue_config` instead, it must set `egress_pool`.

!!! warning
    Do not define **both** the queue helper and your own `get_queue_config` handler that ends in a blanket `return kumo.make_queue_config {}`. The trailing default overrides the helper and you get `unspecified`. Use one or the other.

**2. The tenant points at a pool that isn't defined.**

If a tenant references `pool-2` but `pool-2` does not exist in `sources.toml`, delivery falls back to the host default. Confirm the pool exists and that its IPs are actually plumbed on the host (`ip addr`).

```toml
# sources.toml
[source."ip-1"]
source_address = "10.0.0.1"
ehlo_domain = "mta1.example.com"

[pool."pool-1"]
[pool."pool-1"."ip-1"]
```

```toml
# queues.toml
[tenant.'mytenant']
egress_pool = 'pool-1'
```

## See also

* [Configuring Sending IPs](../userguide/configuration/sendingips.md)
* [Configuring Queue Management](../userguide/configuration/queuemanagement.md)
