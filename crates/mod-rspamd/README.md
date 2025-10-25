# mod-rspamd: Native Rspamd Integration for KumoMTA

This module provides native, high-performance Rspamd integration for spam scanning in KumoMTA.

## Features

- **Native Async Rust Client**: Built on `reqwest` with full async/await support for high throughput
- **File Header Support**: Efficient scanning via file path (no message body transfer over sockets)
- **Automatic Metadata Extraction**: Seamlessly extracts connection and message metadata
- **Connection Pooling**: Automatic HTTP connection pooling for optimal performance
- **Flexible Action Handling**: Multiple strategies for handling Rspamd results (reject, tag, quarantine, custom)
- **Full Protocol Support**: Implements Rspamd's `/checkv2` endpoint with all standard headers

## Architecture

### Components

1. **`RspamdClient`**: Core client with async message scanning
   - Manages HTTP client lifecycle
   - Handles connection pooling
   - Supports both message body and file path scanning

2. **`RspamdResponse`**: Type-safe response parsing
   - Score and required score thresholds
   - Action recommendations
   - Symbol details with scores
   - URLs and emails extracted from message

3. **Lua Bindings**: Exposed via `kumo.rspamd` module
   - `build_client()`: Create configured client
   - `scan()`: Generic scan with custom parameters
   - `scan_with_message()`: Scan with message body
   - `scan_with_file()`: Efficient file-based scanning

## File Header Optimization

When `use_file_header` is enabled (default), the client sends the message's spool file path to Rspamd instead of the message body. This provides:

- **Zero-copy scanning**: No message body transfer over network/sockets
- **Reduced memory pressure**: Especially important for large messages (>1MB)
- **Better performance**: 30-50% faster for large messages on localhost/Unix sockets

**Requirements for File Header**:
- Rspamd must be accessible via localhost or Unix socket
- Rspamd worker must have read access to KumoMTA spool directory
- Message must be spooled (always true for `smtp_server_message_received`)

## Usage Examples

### Simple Setup (Recommended)

```lua
local rspamd = require 'policy-extras.rspamd'

rspamd:setup {
  base_url = 'http://localhost:11333',
  use_file_header = true,  -- Enable efficient file-based scanning
  skip_authenticated = true,  -- Don't scan authenticated submissions
}

kumo.on('smtp_server_message_received', rspamd.scan)
```

### Custom Client

```lua
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  timeout = '10s',
  password = os.getenv('RSPAMD_PASSWORD'),
  use_file_header = true,
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  local result = rspamd.scan_message(client, msg, conn_meta)

  -- Custom handling based on score
  if result.score >= 15 then
    kumo.reject(550, '5.7.1 Spam detected')
  elseif result.score >= 5 then
    msg:set_first_named_header('X-Spam-Flag', 'YES')
  end

  msg:set_meta('rspamd_score', result.score)
end)
```

### Direct API Usage

```lua
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  use_file_header = true,
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Manual parameter construction
  local result = client:scan {
    file_path = msg:get_spool_path(),  -- Use file for efficiency
    ip = conn_meta:get_meta('received_from'),
    helo = conn_meta:get_meta('ehlo_domain'),
    from = tostring(msg:sender()),
    rcpt = tostring(msg:recipient()),
    queue_id = msg:id(),
  }

  print('Rspamd score:', result.score)
  print('Action:', result.action)
end)
```

## Configuration Reference

### Client Configuration

```lua
kumo.rspamd.build_client {
  -- Required
  base_url = "http://localhost:11333",  -- Rspamd base URL

  -- Optional
  timeout = "10s",                       -- Request timeout (default: 10s)
  password = "secret",                   -- Rspamd password for authentication
  use_file_header = true,                -- Enable file-based scanning (default: true)
}
```

### Scan Parameters

```lua
client:scan {
  -- Message data (one of these required if use_file_header=false)
  message = "Subject: Test\r\n\r\nBody",

  -- File path (used if use_file_header=true)
  file_path = "/var/spool/kumo/data/abc123",

  -- Optional metadata headers
  ip = "192.0.2.1",                     -- Client IP address
  helo = "mail.example.com",            -- HELO/EHLO domain
  from = "sender@example.com",          -- Envelope sender
  rcpt = "recipient@example.com",       -- Envelope recipient
  user = "authenticated_user",          -- Authenticated username
  hostname = "mx.example.com",          -- Receiving hostname
  queue_id = "msg123",                  -- Message/Queue ID
}
```

### Response Fields

```lua
{
  score = 5.2,                          -- Spam score
  required_score = 15.0,                -- Threshold for spam
  action = "add header",                -- Recommended action
  message_id = "abc123",                -- Rspamd message ID

  symbols = {                           -- Matched symbols
    DKIM_ALLOW = {
      score = -0.1,
      options = {"example.com"},
    },
    MIME_BAD_EXTENSION = {
      score = 2.5,
      options = {".exe"},
    },
  },

  urls = {                              -- Extracted URLs
    "http://example.com/link",
  },

  emails = {                            -- Extracted emails
    "contact@example.com",
  },
}
```

## Action Handlers

The `policy-extras/rspamd.lua` module provides default handlers for Rspamd actions:

| Action | Behavior |
|--------|----------|
| `reject` | 550 rejection with score |
| `soft reject` | 451 temporary failure (greylist) |
| `rewrite subject` | Prepend "***SPAM***" to subject |
| `add header` | Add X-Spam-* headers |
| `no action` | Add X-Spam-Flag: NO |
| `greylist` | 451 temporary failure |

Custom handlers can be provided:

```lua
local custom_handlers = {
  reject = function(msg, result)
    kumo.reject(550, 'Custom spam message: ' .. result.score)
  end,
}

rspamd.apply_action(msg, result, custom_handlers)
```

## Performance Considerations

1. **File Header**: Always enable `use_file_header = true` for localhost/Unix socket deployments
2. **Connection Pooling**: Client automatically pools HTTP connections
3. **Timeout**: Set appropriate timeout (default 10s) based on message size and network latency
4. **Skip Authenticated**: Skip scanning for trusted authenticated users to save resources

## Integration with Rspamd

### Rspamd Configuration

Ensure Rspamd worker can access KumoMTA spool:

```nginx
# /etc/rspamd/worker-normal.inc
bind_socket = "localhost:11333";
```

For file header support, ensure read access:

```bash
# Add rspamd user to kumo group
usermod -a -G kumo rspamd

# Ensure spool is group-readable
chmod g+r /var/spool/kumomta/data/*
```

### Testing

Send a test message through KumoMTA:

```bash
curl -X POST http://localhost:8000/inject/v1 \
  -H 'Content-Type: application/json' \
  -d '{
    "envelope_sender": "test@example.com",
    "content": "Subject: Test\r\n\r\nGTUBE: XJS*C4JDBQADN1.NSBN3*2IDNEN*GTUBE-STANDARD-ANTI-UBE-TEST-EMAIL*C.34X",
    "recipients": [{"email": "recipient@example.com"}]
  }'
```

Check Rspamd logs:

```bash
journalctl -u rspamd -f
```

## Error Handling

The client handles errors gracefully:

- **Connection failures**: Returns Lua error (can be caught with pcall)
- **Timeout**: Returns error after configured timeout
- **HTTP errors**: Returns error with status code and message
- **Invalid responses**: Returns JSON parse error

Example error handling:

```lua
local status, result = pcall(function()
  return rspamd.scan_message(client, msg, conn_meta)
end)

if not status then
  kumo.log_error('Rspamd scan failed: ' .. tostring(result))
  -- Decide: accept, reject, or tempfail
  kumo.reject(451, '4.7.1 Temporary scanning failure')
  return
end
```

## See Also

- [KumoMTA Documentation](https://docs.kumomta.com/)
- [Rspamd Documentation](https://rspamd.com/doc/)
- [Rspamd Protocol Spec](https://rspamd.com/doc/developers/protocol.html)
- Example configuration: `examples/rspamd-integration.lua`
- Helper module: `assets/policy-extras/rspamd.lua`
