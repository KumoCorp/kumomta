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
  proxy.start_proxy_listener {
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

## Available Endpoints

The HTTP listener exposes the following endpoints:

| Endpoint | Description |
|----------|-------------|
| `/metrics` | Prometheus-compatible metrics in text format |
| `/metrics.json` | Metrics in JSON format |
| `/proxy/status` | Simple health check endpoint |
| `/api/admin/set_diagnostic_log_filter/v1` | Runtime log filter adjustment |
| `/rapidoc` | Interactive API documentation |
| `/api-docs/openapi.json` | OpenAPI specification |

## Metrics

The proxy server exposes Prometheus-compatible metrics for monitoring.
All metrics are automatically documented via the `declare_metric!` macro.

The metrics include:
- `proxy_connections_accepted_total` - Total incoming connections accepted by the proxy
- `proxy_connections_failed_total` - Connections that failed during handshake or proxying
- `proxy_connections_completed_total` - Proxy sessions that completed successfully
- `proxy_active_connections` - Current number of active proxy connections
- `proxy_bytes_client_to_dest_total` - Total bytes transferred from client to destination
- `proxy_bytes_dest_to_client_total` - Total bytes transferred from destination to client
- `proxy_outbound_connections_total` - Outbound connections made to destinations
- `proxy_tls_handshake_failures_total` - TLS handshake failures

Access these metrics via the `/metrics` endpoint in Prometheus text format or
`/metrics.json` for JSON format.

## API Documentation

The HTTP listener automatically provides an OpenAPI specification at
`/api-docs/openapi.json` and interactive documentation at `/rapidoc`.

You can also generate the OpenAPI spec by running:

```bash
proxy-server --dump-openapi-spec > proxy-server.openapi.json
```
