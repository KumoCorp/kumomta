# spool_message_enumerated

```lua
kumo.on('spool_message_enumerated', function(message) end)
```

Called by the spool layer during spool enumeration during server startup.

When kumod starts, after the `init` event has triggered, the spool subsystem
begins enumeration of messages to build up the queues.

For each message discovered in the spool, the `spool_message_enumerated`
event will fire prior to inserting the message into a queue.

This event gives the operator the ability to handle situations such as the
removal of a queues/paths/sources by allowing you to re-assign the queue
meta value to place the message into a different queue from that the one
that was selected during the original reception of the message.

Errors raised during the evaluation of this hook will prevent the server
from completing startup.
