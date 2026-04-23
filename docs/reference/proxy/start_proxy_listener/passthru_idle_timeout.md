# passthru_idle_timeout

{{since('dev')}}

Tears down a proxied passthru session when no data has been transferred in
either direction for the specified duration.

## Motivation

Some ISPs and remote mail servers (notably QQ.com's rate-limiter) respond to
connection-rate pressure by silently holding the TCP connection open — they
send neither data nor a FIN/RST — instead of cleanly refusing. Without an
idle watchdog, the two file descriptors for such a session remain open for the
lifetime of the process, gradually exhausting the file-descriptor table and
occupying proxy worker slots.

`passthru_idle_timeout` solves this by running a per-direction read watchdog
during the passthru phase: if an individual `read` call on either the client
or the remote side exceeds the timeout, the session is torn down and both
sockets are closed.

## Interaction with `use_splice`

Because `splice(2)` does not expose per-direction byte progress to userspace,
**setting `passthru_idle_timeout` forces the userspace copy path regardless
of `use_splice`**. The splice optimisation only applies when the idle timeout
is disabled (set to `none`).

## Default

`5 minutes` (300 seconds). This means sessions where neither side sends any
data for five minutes will be terminated. Adjust this if your workload includes
legitimate long-lived idle connections.

## Examples

Use the default (5 minutes):

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
}
```

Shorten to 2 minutes for an environment with an aggressive FD budget:

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  passthru_idle_timeout = '2 minutes',
}
```

Disable entirely (restores pre-{{since('dev')}} behaviour — FD leak risk):

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
  passthru_idle_timeout = 'none',
}
```
