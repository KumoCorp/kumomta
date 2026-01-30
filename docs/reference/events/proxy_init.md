# proxy_init

{{since('dev')}}

The `proxy_init` event is triggered when the proxy server starts up.

!!! note
    This event is only available to the `proxy-server` executable.

This is where you should configure your proxy listeners and any other
initialization tasks for the proxy server.

```lua
kumo.on('proxy_init', function()
  -- Start SOCKS5 proxy listener
  kumo.start_proxy_listener {
    listen = '0.0.0.0:1080',
    timeout = '60 seconds',
  }

  -- Start HTTP listener for metrics and administration
  kumo.start_proxy_http_listener {
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
| [kumo.start_proxy_http_listener](../kumo/start_proxy_http_listener/index.md) | Start an HTTP listener for metrics |

## Connection Logging

When connections are established through the proxy, structured log messages
are emitted with the following fields:

### proxy_connection (on connect)

| Field | Description |
|-------|-------------|
| `timestamp` | RFC3339 formatted timestamp |
| `connection_id` | Unique UUID for this connection |
| `origin_ip` | Client/peer IP address |
| `proxy_ip` | Proxy listener address |
| `source_ip` | Outbound source address |
| `destination_ip` | Target destination address |
| `username` | Authenticated username (if any) |
| `result` | Connection result status |

### proxy_disconnect (on disconnect)

| Field | Description |
|-------|-------------|
| `timestamp` | RFC3339 formatted timestamp |
| `connection_id` | Unique UUID for this connection |
| `origin_ip` | Client/peer IP address |
| `proxy_ip` | Proxy listener address |
| `source_ip` | Outbound source address |
| `destination_ip` | Target destination address |
| `username` | Authenticated username (if any) |
| `bytes_sent` | Bytes sent to client |
| `bytes_received` | Bytes received from client |
| `duration_secs` | Session duration in seconds |

## Example Log Output

```
2026-01-30T10:15:32.456Z INFO proxy_connection timestamp="2026-01-30T10:15:32.456+00:00" connection_id="a1b2c3d4-..." origin_ip="192.168.1.100:45678" proxy_ip="0.0.0.0:1080" source_ip="10.0.0.1:54321" destination_ip="93.184.216.34:25" username="-" result="connected"

2026-01-30T10:16:05.789Z INFO proxy_disconnect timestamp="2026-01-30T10:16:05.789+00:00" connection_id="a1b2c3d4-..." origin_ip="192.168.1.100:45678" proxy_ip="0.0.0.0:1080" source_ip="10.0.0.1:54321" destination_ip="93.184.216.34:25" username="-" bytes_sent=1024 bytes_received=2048 duration_secs=33.333
```
