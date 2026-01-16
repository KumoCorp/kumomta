# timeout

Specifies the connection timeout duration. If a client does not complete
the SOCKS5 handshake or send data within this duration, the connection
will be closed.

The default is 60 seconds.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  timeout = '30 seconds',
}
```

