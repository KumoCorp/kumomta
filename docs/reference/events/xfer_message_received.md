# xfer_message_received

{{since('2025.12.02-67ee9e96')}}

```lua
kumo.on('xfer_message_received', function(message, auth_info) end)
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

The `auth_info` parameter is an [AuthInfo](../kumo.aaa/auth_info.md) object
that can be used to implement more granular access policies. {{since('dev',
inline=True)}}
