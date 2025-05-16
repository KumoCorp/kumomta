# num_attempts

```lua
message:num_attempts()
```

{{since('2024.06.10-84e84b89')}}

This method returns the number of delivery attempts that have been made for
this message.

Each time a delivery attempt is made for a message, an internal attempt
counter is incremented by one.

!!! note
    The number of attempts is not persistently spooled with the message,
    in order to reduce the IOPS impact of a large number of transient
    delivery attempts. The counter is stored in memory instead.
    If you restart kumod then an approximation of the number of attempts
    will be inferred when the message is spooled in, based on its age
    and the delivery parameters of the corresponding scheduled queue.
