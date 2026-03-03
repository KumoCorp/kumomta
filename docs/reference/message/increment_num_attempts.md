# increment_num_attempts

```lua
message:increment_num_attempts()
```

{{since('dev')}}

This method increments the number of attempts recorded in the message.

You will not ordinarily need to call this method, as each time a delivery
attempt is made for a message, an internal attempt counter is incremented by
one.

You might consider calling this if you are performing custom automated
queue rebinding together with message transfers.

!!! note
    The number of attempts is not persistently spooled with the message,
    in order to reduce the IOPS impact of a large number of transient
    delivery attempts. The counter is stored in memory instead.
    If you restart kumod then an approximation of the number of attempts
    will be inferred when the message is spooled in, based on its age
    and the delivery parameters of the corresponding scheduled queue.

