# xfer_message_received

{{since('dev')}}

```lua
kumo.on('xfer_message_received', function(message) end)
```

Called during reception of a message transfer (xfer) from another kumomta,
just prior to inserting the message into the appropriate queue.

This event provides the opportunity to assess and otherwise update the message
metadata to make it suitable for processing on the current node.

For example, you may choose to alter queue related metadata items to match the
current state of the world on the receiving server.

Any errors that are raised by the event handler will cause the message xfer to
transiently fail; the sending side will retry it according to its retry
schedule.

