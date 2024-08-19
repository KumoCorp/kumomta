# max_message_rate

Optional string.

Specifies the maximum permitted rate at which messages can be delivered
from this source to the corresponding destination site.

The throttle is specified the same was as for `max_connection_rate` above.

If the throttle is exceeded and the delay before the current message can be
sent is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.

This option is distinct from [the scheduled queue
max_message_rate](../make_queue_config/max_message_rate.md) option in that the
scheduled queue option applies to a specific scheduled queue, whilst this
egress path option applies to the ready queue for a specific egress path,
through which multiple scheduled queues send out to the internet.

If you have configured `max_message_rate` both here and in a scheduled queue,
the effective maximum message rate will be the lesser of the two values; both
constraints are applied independently from each other at different stages
of processing.


