# kumo.start_proxy_http_listener

{{since('dev')}}

```lua
kumo.start_proxy_http_listener { PARAMS }
```

Configure and start an HTTP service for the proxy server.

!!! note
    This function is only available to the `proxy-server` executable.

This function should be called only from inside your
[proxy_init](../../events/proxy_init.md) event handler.

The HTTP listener provides access to Prometheus-compatible metrics for
monitoring the proxy server, including connection counts, bytes transferred,
and active connections.

```lua
kumo.on('proxy_init', function()
  -- Start the SOCKS5 proxy
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
  }

  -- Start the HTTP listener for metrics
  kumo.start_proxy_http_listener {
    listen = '0.0.0.0:8080',
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)
```

## Endpoints

The HTTP listener provides the following endpoints:

| Endpoint | Description |
|----------|-------------|
| `/metrics` | Prometheus text format metrics |
| `/metrics.json` | JSON format metrics |
| `/proxy/status` | Simple health check endpoint |
| `/api/admin/set_diagnostic_log_filter/v1` | Change diagnostic log filter |
| `/rapidoc` | Interactive API documentation |

## Available Metrics

The following Prometheus metrics are exposed:

| Metric | Type | Description |
|--------|------|-------------|
| `proxy_connections_accepted_total` | Counter | Total number of incoming connections accepted |
| `proxy_connections_failed_total` | Counter | Total number of connections that failed |
| `proxy_connections_completed_total` | Counter | Total number of successful proxy sessions |
| `proxy_active_connections` | Gauge | Current number of active proxy connections |
| `proxy_bytes_received_total` | Counter | Total bytes received from clients |
| `proxy_bytes_sent_total` | Counter | Total bytes sent to clients |
| `proxy_outbound_connections_total` | Counter | Total outbound connections by destination |
| `proxy_tls_handshake_failures_total` | Counter | Total TLS handshake failures (when TLS is enabled) |

All metrics are labeled with `listener` (the listener address). The `proxy_outbound_connections_total` metric is additionally labeled with `destination`.

!!! warning "High Cardinality Metric"
    The `proxy_outbound_connections_total` metric tracks connections by destination IP address.
    If your proxy connects to many unique destinations, this can create high cardinality
    in Prometheus, which may impact memory usage and query performance. This metric uses
    a pruning counter registry to mitigate this, but monitor cardinality in production environments.

## Example Prometheus Query

To see active connections per listener:

```promql
proxy_active_connections{listener="0.0.0.0:1080"}
```

To calculate connection rate:

```promql
rate(proxy_connections_accepted_total[5m])
```

`PARAMS` is a lua table that accepts the same keys as
[kumo.start_http_listener](../start_http_listener/index.md).

## HTTP Listener Parameters { data-search-exclude }

