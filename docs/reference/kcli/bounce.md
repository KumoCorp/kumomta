---
tags:
  - bounce
---
# kcli bounce


Administratively bounce messages in matching queues.

Each individual message that is bounced will generate a log record capturing the event and then be removed from the spool.

Make sure that you mean it, as there is no going back!

The bounce will be applied immediately to queued messages, (asynchronously with respect to the command!) and the directive will remain in effect for the duration specified, causing newly received messages or messages that were in a transient state at the time the directive was received, to also be bounced as they are placed back into the matching queue(s).

The totals printed by this command are often under-reported due to the asynchronous nature of the action.


**Usage:** `kcli bounce [OPTIONS] --reason <REASON> <--domain <DOMAIN>|--routing-domain <ROUTING_DOMAIN>|--campaign <CAMPAIGN>|--tenant <TENANT>|--everything|--queue <QUEUE>>`

## Options


* `--domain <DOMAIN>` — The domain name to match. If omitted, any domains will match!

* `--routing-domain <ROUTING_DOMAIN>` — The routing_domain name to match. If omitted, any routing domain will match!

* `--campaign <CAMPAIGN>` — The campaign name to match. If omitted, any campaigns will match!

* `--tenant <TENANT>` — The tenant name to match. If omitted, any tenant will match!

* `--queue <QUEUE>` — Bounce specific scheduled queue names using their exact queue name(s). Can be specified multiple times

* `--reason <REASON>` — The reason to log in the delivery logs (each matching message will bounce with an AdminBounce record) as well as in the list of bounces

* `--everything` — Purge all queues

* `--suppress-logging` — Do not generate AdminBounce delivery logs

* `--duration <DURATION>` — The duration over which matching messages will continue to bounce. The default is '5m'



