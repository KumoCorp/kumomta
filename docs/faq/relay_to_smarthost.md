---
description: "Relay mail through a smarthost or another SMTP server by setting a routing_domain, with notes on IP literals, MX overrides, and authentication."
---

# How Do I Relay Mail Through a Smarthost or Another SMTP Server?

To "smarthost" (route messages through another server instead of doing direct-to-MX delivery), set a routing domain for the message. KumoMTA still performs MX resolution, just against the routing domain instead of the recipient's domain.

## Per-message, at reception

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('routing_domain', 'my.smarthost.com')
end)
```

This preserves the original destination domain, tenant, and campaign, and creates separate Scheduled Queues (e.g. `example.com!my.smarthost.com`) so each destination can be managed independently.

## Host names vs. IP literals

A host name is given without brackets: `my.smarthost.com`. An IP literal must be bracketed per the SMTP spec: `[10.0.0.1]` or `[IPv6:::1]`.

## Sending to infrastructure with no MX

To relay to internal hosts that have no MX records, override the MX list for the Scheduled Queue in `get_queue_config` via `make_queue_config { protocol = { smtp = { mx_list = { ... } } } }`.

## Authentication to the smarthost

Put the smarthost's SMTP-AUTH credentials and TLS options on that destination's egress path (in your shaping configuration), and remember to assign an egress pool, or delivery will be "undetermined".

!!! note
    The queue helper does not support wildcards/regex; for "relay everything for a tenant" set `routing_domain` in Lua rather than in `queues.toml`.

## See also

* [Configuring Message Routing](../userguide/policy/routing.md)
