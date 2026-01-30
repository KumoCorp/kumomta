# proxy_init

{{since('dev')}}

The `proxy_init` event is triggered when the proxy server starts up.

!!! note
    This event is only available to the `proxy-server` executable.

This is where you should configure your proxy listeners and any other
initialization tasks for the proxy server.

```lua
local kumo = require 'kumo'
local proxy = require 'proxy'

kumo.on('proxy_init', function()
  -- Start SOCKS5 proxy listener
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
    timeout = '60 seconds',
  }

  -- Start HTTP listener for metrics and administration
  proxy.start_http_listener {
    listen = '0.0.0.0:8080',
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)
```

## Available Functions

The following functions are available during the `proxy_init` event:

| Function | Description |
|----------|-------------|
| [kumo.start_proxy_listener](../kumo/start_proxy_listener/index.md) | Start a SOCKS5 proxy listener |
| [proxy.start_http_listener](../proxy/start_http_listener.md) | Start an HTTP listener for metrics |
