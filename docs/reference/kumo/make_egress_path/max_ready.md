# max_ready

Specifies the maximum number of messages that can be in the *ready queue*.
The ready queue is the set of messages that are immediately eligible for delivery.

If a message is promoted from its delayed queue to the ready queue and it would
take the size of the ready queue above *max_ready*, the message will be delayed
by a randomized interval of up to 60 seconds and placed back into the scheduled
queue before being considered again.

Moving a message from *ready* to *scheduled* as a result of hitting this limit
may trigger disk IO to save the content of the message if the message was
received with deferred spooling enabled.  In addition, other in-memory state
is discarded to reduce memory utilization, and it will need to be re-loaded
from the spool when the message is tried again later.

The default for `max_ready` is 1024 messages.

Raising the limit will increase RAM utilization in exchange for decreasing
the IO load to your spool storage.


