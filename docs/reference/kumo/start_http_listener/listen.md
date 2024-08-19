# listen

Specifies the local IP and port number to which the HTTP service
should bind and listen.

Use `0.0.0.0` to bind to all IPv4 addresses.

```lua
kumo.start_http_listener {
  listen = '0.0.0.0:80',
}
```


