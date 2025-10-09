--[[
# Rspamd Integration Example for KumoMTA

This example demonstrates how to integrate Rspamd spam scanning with KumoMTA.

## Prerequisites

1. Rspamd must be installed and running
2. Default Rspamd listens on localhost:11333
3. Configure Rspamd to allow scanning from localhost

## Features Demonstrated

- Native async Rspamd client
- **File header optimization** for local scanning (avoids sending message body)
- ZSTD compression for faster transmission (enabled by default)
- HTTPCrypt encryption for secure communication
- Automatic metadata extraction (IP, HELO, sender, recipient)
- Multiple action handling strategies
- Custom score-based actions

## Testing

Start Rspamd:
```bash
sudo systemctl start rspamd
# or
rspamd -c /etc/rspamd/rspamd.conf
```

Send a test message:
```bash
curl -X POST http://localhost:8000/inject/v1 \
  -H 'Content-Type: application/json' \
  -d '{
    "envelope_sender": "sender@example.com",
    "content": "Subject: Test\r\n\r\nThis is a test message",
    "recipients": [{"email": "recipient@example.com"}]
  }'
```

]]

local kumo = require 'kumo'
local rspamd = require 'policy-extras.rspamd'

--[[
# Example 1: Simple Setup with Automatic Actions

This follows Rspamd's recommendations:
- reject → 550 error
- soft reject → 451 error (greylist)
- rewrite subject → prepend ***SPAM***
- add header → add X-Spam-* headers
- no action → add X-Spam-Flag: NO
]]

-- Simple setup - uncomment to use
--[[
rspamd:setup {
  base_url = 'http://localhost:11333',
  use_file_path = true,  -- Use File header optimization (recommended for localhost)
  zstd = true,  -- Enable compression (default: true)
  skip_authenticated = true,  -- Don't scan messages from authenticated users
}

kumo.on('smtp_server_message_received', rspamd.scan)
]]

--[[
# Example 2: Tag-Only Mode

Add headers but never reject:
]]

--[[
rspamd:setup {
  base_url = 'http://localhost:11333',
  action = 'tag',  -- Only add headers, never reject
  use_file_path = true,  -- File header optimization
  zstd = true,  -- Compression enabled by default
}

kumo.on('smtp_server_message_received', rspamd.scan)
]]

--[[
# Example 3: Quarantine Mode

High-scoring spam goes to quarantine instead of bouncing:
]]

--[[
rspamd:setup {
  base_url = 'http://localhost:11333',
  action = 'quarantine',
  quarantine_threshold = 5.0,  -- Score above which to quarantine
  quarantine_queue = 'spam_quarantine',
  use_file_path = true,  -- File header optimization
  zstd = true,  -- Compression enabled
}

kumo.on('smtp_server_message_received', rspamd.scan)
]]

--[[
# Example 4: Custom Action Handling

Full control over actions based on Rspamd results:
]]

--[[
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  timeout = '10s',
  zstd = true,  -- Compression enabled
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Skip authenticated users
  if conn_meta:get_meta 'authn_id' then
    return
  end

  -- Scan the message with file path optimization
  local result = rspamd.scan_message(client, msg, conn_meta, {use_file_path = true})

  -- Log the result
  kumo.log_info(
    string.format(
      'Rspamd scan: score=%.2f action=%s',
      result.score,
      result.action
    )
  )

  -- Custom score-based handling
  if result.score >= 15 then
    -- Definite spam - reject
    kumo.reject(550, string.format('5.7.1 Spam detected (score: %.2f)', result.score))
  elseif result.score >= 5 then
    -- Probable spam - tag and route to quarantine
    msg:set_first_named_header('X-Spam-Flag', 'YES')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
    msg:set_meta('queue', 'quarantine')
  elseif result.score >= 0 then
    -- Uncertain - just tag
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  else
    -- Ham
    msg:set_first_named_header('X-Spam-Flag', 'NO')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end

  -- Store full results for logging
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  -- Store top symbols for analysis
  local top_symbols = {}
  for name, data in pairs(result.symbols or {}) do
    if data.score and data.score > 1 then
      table.insert(top_symbols, name)
    end
  end
  if #top_symbols > 0 then
    msg:set_meta('rspamd_symbols', kumo.json_encode(top_symbols))
  end
end)
]]

--[[
# Example 5: Advanced Setup with HTTPCrypt Encryption

Use HTTPCrypt for encrypted communication with Rspamd:

Generate encryption keypair on Rspamd server:
```bash
rspamadm keypair -u
```
This will output a public key (for Rspamd config) and private key (for KumoMTA).

Add to Rspamd worker config (/etc/rspamd/worker-normal.inc):
```
keypair {
  pubkey = "your-public-key-here";
  privkey = "your-private-key-here";
}
```
]]

local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  timeout = '10s',
  password = os.getenv 'RSPAMD_PASSWORD', -- Optional password
  zstd = true, -- Enable compression (default: true)
  encryption_key = os.getenv 'RSPAMD_ENCRYPTION_KEY', -- HTTPCrypt encryption key
  retries = 2, -- Retry failed requests
}

-- Custom action handlers
local custom_handlers = {
  -- Override reject to use custom message
  reject = function(msg, result)
    local symbols = {}
    for name, _ in pairs(result.symbols or {}) do
      table.insert(symbols, name)
    end

    kumo.reject(
      550,
      string.format(
        '5.7.1 Message rejected due to spam characteristics. Score: %.2f (threshold: %.2f). Symbols: %s',
        result.score,
        result.required_score,
        table.concat(symbols, ', ')
      )
    )
  end,

  -- Custom handling for soft reject (greylist)
  ['soft reject'] = function(msg, result)
    -- Instead of greylisting, accept but quarantine
    msg:set_meta('queue', 'greylist_review')
    msg:set_first_named_header('X-Greylist', 'YES')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end,
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Skip authenticated users
  if conn_meta:get_meta 'authn_id' then
    return
  end

  -- Scan with file path optimization
  local result = rspamd.scan_message(client, msg, conn_meta, {use_file_path = true})

  -- Apply custom actions
  rspamd.apply_action(msg, result, custom_handlers)

  -- Always store metadata for logs
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)

--[[
# Example 6: Direct API Usage (Low-Level)

For maximum control, use the client API directly:
]]

--[[
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  zstd = true,  -- Compression enabled by default
  encryption_key = os.getenv 'RSPAMD_KEY',  -- Optional HTTPCrypt
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Build scan metadata manually
  local metadata = {
    ip = conn_meta:get_meta 'received_from',
    helo = conn_meta:get_meta 'ehlo_domain',
    from = tostring(msg:sender()),
    rcpt = { tostring(msg:recipient()) },
    queue_id = msg:id(),
    hostname = conn_meta:get_meta 'hostname',
    -- Optional: use file_path for local scanning (avoids body transfer)
    file_path = msg:get_meta 'spool_path',
  }

  -- Get message data (not transmitted if file_path is set)
  local message_data = msg:get_data()

  -- Scan with compression and optional encryption
  -- If file_path is set, Rspamd reads from disk instead
  local result = client:scan(message_data, metadata)

  -- Process results as needed
  print('Score:', result.score)
  print('Action:', result.action)
  for symbol, data in pairs(result.symbols or {}) do
    print('Symbol:', symbol, 'Score:', data.score or 0)
  end
end)
]]

-- Basic init configuration (adjust for your environment)
kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2525',
    relay_hosts = { '127.0.0.1' },
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    trusted_hosts = { '127.0.0.1', '::1' },
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
  }
end)

kumo.on('get_egress_path_config', function(domain)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
  }
end)
