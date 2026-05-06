# tcp_keepalive

Configures TCP keepalive on the inbound (accepted client) and outbound
(proxied destination) sockets handled by the proxy listener.  When TCP
keepalive is enabled, the kernel periodically probes an idle connection;
if the peer stops responding, the socket is closed with an error rather
than remaining open indefinitely.  This bounds the lifetime of stale or
unresponsive connections and prevents them from accumulating against the
process file-descriptor limit.

Accepted keys:

* `enabled` — boolean. When `false`, no keepalive is configured and the
  kernel default (typically off) is used. Default `true`.
* `time` — duration. Idle period before the first keepalive probe is sent.
  Default `'5 minutes'`.
* `interval` — duration. Interval between subsequent probes.
  Default `'30 seconds'`.
* `retries` — integer. Number of unanswered probes before the connection
  is considered dead and reported to the application as an error.
  Default `3`.

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  tcp_keepalive = {
    time = '2 minutes',
    interval = '15 seconds',
    retries = 4,
  },
}
```

To disable keepalive entirely:

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  tcp_keepalive = {
    enabled = false,
  },
}
```
