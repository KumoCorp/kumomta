# listen_backlog

{{since('dev')}}

Sets the accept-queue depth requested of the kernel via `listen(2)` for this
listener. This is the number of fully established connections the kernel will
hold waiting to be accepted. When the queue is full, additional connection
attempts are dropped and the client must retry.

When this option is not set, the kernel maximum is requested. On Linux that is
the value of the `net.core.somaxconn` sysctl.

The kernel silently clamps the requested value to the `net.core.somaxconn`
sysctl. Setting `listen_backlog` higher than that sysctl has no further effect.
Raise the sysctl itself to get a larger queue.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  listen_backlog = 1024,
}
```

Prior to this option the backlog was fixed at 128 regardless of
`net.core.somaxconn`.
