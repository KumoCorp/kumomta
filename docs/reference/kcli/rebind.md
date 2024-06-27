# kcli rebind


Rebind messages from matching queues into different queue(s).

Rebinding works first by selecting the set of scheduled queues based on matching criteria that you specify via the `--domain`, `--routing-domain`, `--campaign`, `--tenant` and/or `--everything` options.

Each matching queue has its messages removed and assessed by the rebinding logic.

If `--trigger-rebind-event` is in use, each message will be passed to the `rebind_message` event, along with the effective *data* value you specify through a combination of `--data` and/or `--set` parameters.  What actually happens to the message is defined solely by the logic in your `rebind_message` event.

Otherwise, each message will merge the key/value pairs that you specified via `--data` and/or `--set` into the metadata of the message.

After this, the message will be re-inserted into the queue subsystem.

If your rebind action caused any of the envelope recipient (in the case of `--trigger-rebind-event`), `queue`, `tenant`, `campaign` or `routing_domain` meta items to be changed, then the message will be placed into a different queue from its original location; in that case, it will be updated so that it is eligible for immediate delivery and an `AdminRebind` log event will be generated to the logs.

If the queue wasn't changed, then the next-due time of the message will remain unchanged, unless you specified `--always-flush`.  In that case, the message will be placed back into its original queue but be eligible for immediate delivery.

If you do not wish to generate `AdminRebind` log entries, then you can use `--suppress-logging`.

Since the number of messages may be very large, and because processing messages may result in a large amount of I/O to load in every matching message's metadata, the total amount of time taken for a rebind request may be too large to feasibly wait for in the context of a simple request/response.

With that in mind, the rebinding action runs asynchronously: aside from any immediate syntax/request formatting issues, this command will immediately return with no further status indication.

Errors will be reported in the diagnostic log.

## Examples

Move messages from the "example.com" queue to the "foo.com" queue:

kcli rebind --domain example.com --set queue=foo.com

Alternatively:

kcli rebind --domain example.com --data '{"queue": "foo.com"}'


**Usage:** `kcli rebind [OPTIONS] --reason <REASON>`

## Options


* `--domain <DOMAIN>` — The domain name to match. If omitted, any domains will match!

* `--routing-domain <ROUTING_DOMAIN>` — The routing_domain name to match. If omitted, any routing domain will match!

* `--campaign <CAMPAIGN>` — The campaign name to match. If omitted, any campaigns will match!

* `--tenant <TENANT>` — The tenant name to match. If omitted, any tenant will match!

* `--reason <REASON>` — The reason to log in the delivery logs (each matching message will rebind with an AdminRebind record)

* `--always-flush` — Always flush, even if we didn't change the scheduled queue

* `--everything` — Match all queues

* `--suppress-logging` — Do not generate AdminRebind delivery logs

* `--trigger-rebind-event` — Trigger a "rebind_message" event which receives both the message and the data, and then decides what to do to the message. Otherwise, the data will be unconditionally applied to the message metadata

* `--data <DATA>` — Specify a JSON object of key/value pairs which should either be set as meta values, or passed to the rebind_message event (if `--trigger-rebind-event` is in use)

* `--set <KEY=VALUE>` — Set additional key/value pairs. Can be used multiple times



