# kcli xfer


Transfer messages from matching queues to an alternative kumomta node.

The intended purpose of this command is to facilitate manual migration of queues to alternative nodes as part of planned maintenance or part of an orchestrated down-scaling operation.

Xfering works first by selecting the set of scheduled queues based on matching criteria that you specify via the `--domain`, `--routing-domain`, `--campaign`, `--tenant`, `--queue`, and/or `--everything` options.

Each matching queue has its messages drained and the xfer logic will amend the message metadata to capture scheduling and due time information and then place the message into a special `.xfer.kumomta.internal` message transfer queue where it will be immediately eligible to be moved to the destination node.

Upon successful reception on the destination node, the saved scheduling information will be restored to the message and it will be inserted into an appropriate queue on that destination node for delivery at the appropriate time.

Since the number of messages may be very large, and because processing messages may result in a large amount of I/O to load in every matching message's metadata, the total amount of time taken for an xfer request may be too large to feasibly wait for in the context of a simple request/response.

With that in mind, the xfer action runs asynchronously: aside from any immediate syntax/request formatting issues, this command will immediately return with no further status indication.

Errors will be reported in the diagnostic log.

## Examples

Move messages from the "example.com" queue to the kumomta node running an http listener on `http://10.0.0.1:8000`:

``` kcli xfer --domain example.com --target http://10.0.0.1:8000 ```


**Usage:** `kcli xfer --reason <REASON> --target <TARGET> <--domain <DOMAIN>|--routing-domain <ROUTING_DOMAIN>|--campaign <CAMPAIGN>|--tenant <TENANT>|--everything|--queue <QUEUE>>`

## Options


* `--domain <DOMAIN>` — The domain name to match. If omitted, any domains will match!

* `--routing-domain <ROUTING_DOMAIN>` — The routing_domain name to match. If omitted, any routing domain will match!

* `--campaign <CAMPAIGN>` — The campaign name to match. If omitted, any campaigns will match!

* `--tenant <TENANT>` — The tenant name to match. If omitted, any tenant will match!

* `--queue <QUEUE>` — The precise name of a scheduled queue which should match. Can be specified multiple times

* `--reason <REASON>` — Each matching message will be rebound into an appropriate xfer queue, and an AdminRebind log will be generated to trace that the rebind happened.  The reason you specify here will be included in that log record

* `--everything` — Match all queues

* `--target <TARGET>` — Which node to transfer the messages to. This should be an HTTP URL prefix that will reach the HTTP listener on the target node, such as `http://hostname:8000`



