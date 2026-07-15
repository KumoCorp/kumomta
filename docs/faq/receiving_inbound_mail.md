---
description: "Receive inbound mail and process OOB bounces and FBL reports with the listener_domains helper — relay_to, log_oob, and log_arf."
---

# How Do I Receive Inbound Mail and Process OOB Bounces and FBLs?

KumoMTA is a relay: by default it accepts mail from `relay_hosts` and relays it out, but it will not accept inbound mail for a domain unless you tell it to. Use the `listener_domains` helper to declare which domains may be relayed to, and which should be treated as bounce or feedback processors:

```toml
# listener_domains.toml
["example.com"]
# accept and queue inbound mail addressed to example.com
relay_to = true

["bounce.example.com"]
# accept, log, and discard OOB (out-of-band) bounce reports
log_oob = "LogThenDrop"

["fbl.example.com"]
# accept, log, and discard ARF feedback-loop reports
log_arf = "LogThenDrop"
```

## OOB / ARF modes

`log_oob` and `log_arf` accept a string mode: `Ignore`, `LogThenDrop`, or `LogThenRelay`.

!!! warning
    Use the string value `log_oob = "LogThenDrop"`, **not** the boolean `true`. The boolean form logs *and relays* the report, which is rarely what you want and is a common cause of OOB loops.

## Routing inbound mail onward

Because your MX points at KumoMTA, a normal MX lookup for a `relay_to` domain returns KumoMTA itself. To avoid a mail loop, set a routing domain that points at the real mailbox host:

```toml
# queues.toml
[queue.'example.com']
routing_domain = '[10.0.0.1]'
```

## A note on DSNs

KumoMTA logs bounces rather than generating DSN/NDR emails. If you specifically need to emit an RFC 3464 report, use `generate_rfc3464_message`; otherwise integrate via the logs and webhooks.

## See also

* [Configuring Inbound and Relay Domains](../userguide/configuration/domains.md)
* [Configuring Feedback Loop Processing](../userguide/configuration/fbl.md)
* [Configuring Bounce Classification](../userguide/configuration/bounce.md)
