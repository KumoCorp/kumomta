# max_connections

{{since('dev')}}

Specifies the maximum number of concurrent client connections permitted to
this listener. Connections accepted above this limit are closed immediately,
without completing the SOCKS5 handshake, to bound file-descriptor and memory
use.

The default value is `32768`.

Each time a connection is denied due to hitting this limit, the
`proxy_connections_denied_total` counter is incremented for the listener.

```lua
kumo.on('proxy_init', function()
  proxy.start_proxy_listener {
    listen = '0.0.0.0:1080',
    max_connections = 10000,
  }
end)
```
