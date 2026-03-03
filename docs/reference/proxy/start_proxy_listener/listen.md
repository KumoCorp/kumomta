# listen

{{since('dev')}}

Specifies the local IP and port number to which the proxy service
should bind and listen.

Use `0.0.0.0` to bind to all IPv4 addresses.

```lua
proxy.start_proxy_listener {
  listen = '0.0.0.0:1080',
}
```

