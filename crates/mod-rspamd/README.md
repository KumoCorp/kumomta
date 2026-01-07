# mod-rspamd: Native Rspamd Integration for KumoMTA

This module provides native, high-performance Rspamd integration for spam scanning in KumoMTA. All core functionality is implemented in Rust for optimal performance and reliability.

## Features

- **Rust-Native Implementation**: All scanning, header manipulation, and action handling in Rust
- **Automatic Header Management**: X-Spam-* headers added automatically by Rust code
- **Automatic Action Handling**: Built-in support for reject, soft reject, and subject rewriting
- **Automatic Metadata Extraction**: Seamlessly extracts connection and message metadata
- **Connection Pooling**: Automatic HTTP connection pooling for optimal performance
- **Full Protocol Support**: Implements Rspamd's `/checkv2` endpoint with all standard headers

## Architecture

### Components

1. **Rust Core (`kumo.rspamd.scan_message`)**: Main scanning function
   - Extracts message and connection metadata automatically
   - Communicates with Rspamd asynchronously
   - Applies headers and actions based on configuration
   - Returns scan results to Lua for custom logic

2. **`RspamdResponse`**: Type-safe response structure
   - Score and required score thresholds
   - Action recommendations
   - Symbol details with scores
   - URLs and emails extracted from message

3. **Lua API**: Simple configuration-based interface
   - Single function: `kumo.rspamd.scan_message(config, msg)`
   - Config-driven behavior (headers, rejection, subject prefix)
   - Results returned for custom Lua logic

## Usage Examples

### Simple Setup (Recommended)

Scan messages in `smtp_server_data` for efficient per-batch scanning:

```lua
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,     -- Add X-Spam-* headers (default: true)
    reject_spam = true,     -- Reject spam messages (default: false)
    prefix_subject = false, -- Prefix subject with ***SPAM*** (default: false)
  }

  -- Scan and apply actions automatically (all done in Rust)
  local result = kumo.rspamd.scan_message(config, msg)

  -- Store results for logging
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
```

### Custom Score-Based Logic

Add headers automatically but handle rejection in Lua:

```lua
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,  -- Handle rejection in Lua
  }

  local result = kumo.rspamd.scan_message(config, msg)

  -- Custom handling based on score
  if result.score >= 15 then
    kumo.reject(550, string.format('5.7.1 Spam detected (score: %.2f)', result.score))
  elseif result.score >= 5 then
    msg:set_meta('queue', 'quarantine')
  end

  msg:set_meta('rspamd_score', result.score)
end)
```

### Per-Recipient Thresholds

Scan once in `smtp_server_data`, apply per-recipient logic in `smtp_server_message_received`:

```lua
-- Scan once per batch
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,  -- Handle per-recipient
  }

  local result = kumo.rspamd.scan_message(config, msg)
  msg:set_meta('rspamd_score', result.score)
end)

-- Apply per-recipient thresholds
kumo.on('smtp_server_message_received', function(msg)
  local score = msg:get_meta('rspamd_score')
  if not score then return end

  local threshold = 5.0  -- Get from database based on recipient
  if score > threshold then
    kumo.reject(550, string.format('5.7.1 Spam (score: %.2f)', score))
  end
end)
```

## Configuration Reference

### Config Options

```lua
local config = {
  -- Required
  base_url = "http://localhost:11333",  -- Rspamd base URL

  -- Optional (all handled in Rust)
  add_headers = true,                    -- Add X-Spam-* headers (default: true)
  reject_spam = false,                   -- Reject spam messages (default: false)
  reject_soft = false,                   -- Use 4xx instead of 5xx (default: false)
  prefix_subject = false,                -- Prefix subject with ***SPAM*** (default: false)
  timeout = "10s",                       -- Request timeout (default: 10s)
  password = "secret",                   -- Rspamd password for authentication
}

local result = kumo.rspamd.scan_message(config, msg)
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

## Automatic Action Handling

When `add_headers = true`, the Rust implementation automatically adds these headers:

| Header | Content |
|--------|---------|
| `X-Spam-Flag` | YES or NO based on action |
| `X-Spam-Score` | Numeric spam score |
| `X-Spam-Action` | Rspamd's recommended action |
| `X-Spam-Symbols` | List of matched symbols |

When `reject_spam = true`, the following Rspamd actions trigger rejection:

| Action | Response Code | Behavior |
|--------|--------------|----------|
| `reject` | 550 (or 450 if `reject_soft`) | Permanent (or temporary) rejection |
| `soft reject` | 450 | Temporary failure (greylist) |
| `greylist` | 450 | Temporary failure |

When `prefix_subject = true` and action is `rewrite subject`, the subject is prefixed with `***SPAM***`.

All of this header and action handling is performed in Rust for optimal performance.

## Performance Considerations

1. **Use smtp_server_data**: Scan once per batch instead of per recipient
2. **Connection Pooling**: HTTP client automatically pools connections to Rspamd
3. **Timeout**: Default 10s timeout works for most cases
4. **Metadata Extraction**: All metadata (IP, HELO, sender, etc.) extracted automatically in Rust

## Integration with Rspamd

### Rspamd Configuration

Ensure Rspamd is listening and accessible:

```nginx
# /etc/rspamd/worker-normal.inc
bind_socket = "localhost:11333";
```

### Testing

Send a test message with GTUBE spam pattern:

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

The Rust implementation handles errors gracefully and returns them to Lua:

- **Connection failures**: Returns Lua error (can be caught with pcall)
- **Timeout**: Returns error after configured timeout
- **HTTP errors**: Returns error with status code and message
- **Invalid responses**: Returns JSON parse error

Example error handling:

```lua
kumo.on('smtp_server_data', function(msg)
  local status, result = pcall(function()
    local config = { base_url = 'http://localhost:11333' }
    return kumo.rspamd.scan_message(config, msg)
  end)

  if not status then
    kumo.log_error('Rspamd scan failed: ' .. tostring(result))
    -- Decide: accept, reject, or tempfail
    kumo.reject(451, '4.7.1 Temporary scanning failure')
    return
  end

  msg:set_meta('rspamd_score', result.score)
end)
```

## See Also

- [KumoMTA Documentation](https://docs.kumomta.com/)
- [Rspamd Documentation](https://rspamd.com/doc/)
- [Rspamd Protocol Spec](https://rspamd.com/doc/developers/protocol.html)
- Example configuration: `examples/rspamd-integration.lua`
