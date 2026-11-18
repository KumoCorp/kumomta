# listen_backlog

{{since('dev')}}

Sets the accept-queue depth requested of the kernel via `listen(2)` for this
listener. This is the number of fully established connections the kernel will
hold waiting for kumod to accept them. When the queue is full, additional
connection attempts are dropped and the client must retry, which shows up as a
listen overflow and a multi-second connect stall for the client.

When this option is not set, the kernel maximum is requested. On Linux that is
the value of the `net.core.somaxconn` sysctl.

The kernel silently caps the requested value at its own maximum. A value
larger than `net.core.somaxconn` has no effect beyond that ceiling. To go
higher, raise the sysctl.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  listen_backlog = 1024,
}
```

Prior to this option the backlog was fixed at 128 regardless of
`net.core.somaxconn`.
