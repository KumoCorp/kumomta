# proxy.start_http_listener

{{since('dev')}}

```lua
proxy.start_http_listener { PARAMS }
```

Configure and start an HTTP service for the proxy server.

!!! note
    This function is only available to the `proxy-server` executable.

This function should be called only from inside your
[proxy_init](../events/proxy_init.md) event handler.

The HTTP listener provides access to Prometheus-compatible metrics for
monitoring the proxy server, including connection counts, bytes transferred,
and active connections.

```lua
local kumo = require 'kumo'
local proxy = require 'proxy'

kumo.on('proxy_init', function()
  -- Start the SOCKS5 proxy
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
  }

  -- Start the HTTP listener for metrics
  proxy.start_http_listener {
    listen = '0.0.0.0:8080',
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)
```

## Parameters

`PARAMS` accepts the same keys as
[kumo.start_http_listener](../kumo/start_http_listener/index.md).

## API Documentation

The HTTP listener automatically provides an OpenAPI specification at
`/api-docs/openapi.json` and interactive documentation at `/rapidoc`.

You can also generate the OpenAPI spec by running:

```bash
proxy-server --dump-openapi-spec > proxy-server.openapi.json
```
