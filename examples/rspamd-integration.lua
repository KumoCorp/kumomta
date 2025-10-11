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
- **Body rewriting** for message modifications (rspamd-client 0.4+)
- **Milter protocol support** for header modifications (add/remove headers, subject rewriting)
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
# Example 1: Simplified API with Built-in Actions

The kumo.rspamd.scan_message() function provides a streamlined interface
that automatically extracts message metadata and applies default actions.
]]

--[[
kumo.on('smtp_server_message_received', function(msg)
  -- Simple config with default actions
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,     -- Add X-Spam-* headers (default: true)
    reject_spam = true,     -- Reject spam messages (default: false)
    reject_soft = false,    -- Use 4xx instead of 5xx (default: false)
    prefix_subject = false, -- Prefix subject with ***SPAM*** (default: false)
  }

  -- Scan and apply default actions
  local result = kumo.rspamd.scan_message(config, msg)

  -- Result contains the full Rspamd response
  -- Default actions have already been applied based on config
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 1a: Simplified API with Encryption

Using HTTPCrypt encryption with the simplified API.

Generate encryption keypair on Rspamd server:
```bash
rspamadm keypair -u
```
This outputs a public key (for Rspamd config) and private key (for KumoMTA).

Add to Rspamd worker config (/etc/rspamd/worker-normal.inc):
```
keypair {
  pubkey = "your-public-key-here";
  privkey = "your-private-key-here";
}
```
]]

--[[
kumo.on('smtp_server_message_received', function(msg)
  local config = {
    base_url = 'http://localhost:11333',
    encryption_key = os.getenv('RSPAMD_ENCRYPTION_KEY'), -- HTTPCrypt encryption
    password = os.getenv('RSPAMD_PASSWORD'),              -- Optional password
    add_headers = true,
    reject_spam = true,
  }

  local result = kumo.rspamd.scan_message(config, msg)

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 2: Setup Helper with Automatic Actions

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
# Example 3: Tag-Only Mode

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
# Example 4: Quarantine Mode

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
# Example 5: Custom Action Handling

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
    msg:prepend_header('X-Spam-Flag', 'YES')
    msg:prepend_header('X-Spam-Score', tostring(result.score))
    msg:set_meta('queue', 'quarantine')
  elseif result.score >= 0 then
    -- Uncertain - just tag
    msg:prepend_header('X-Spam-Score', tostring(result.score))
  else
    -- Ham
    msg:prepend_header('X-Spam-Flag', 'NO')
    msg:prepend_header('X-Spam-Score', tostring(result.score))
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
# Example 6: Milter Actions - Header Modifications

Rspamd can suggest header modifications via the milter protocol:
- Add headers (X-Spam-*, custom headers, etc.)
- Remove headers
- Rewrite Subject line

Milter actions are applied automatically by default.
]]

--[[
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  use_file_path = true,
  zstd = true,
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  if conn_meta:get_meta 'authn_id' then
    return
  end

  -- Scan the message
  local result = rspamd.scan_message(client, msg, conn_meta, {use_file_path = true})

  -- Milter actions are automatically applied by rspamd.apply_action()
  -- This includes:
  -- - Adding headers from result.milter.add_headers
  -- - Removing headers from result.milter.remove_headers
  -- - Subject rewriting if Rspamd suggests it

  -- Apply actions including milter modifications
  rspamd.apply_action(msg, result)

  -- You can also apply milter actions independently:
  -- rspamd.apply_milter_actions(msg, result)

  -- Log what milter actions were applied
  if result.milter then
    if result.milter.add_headers then
      for header_name, header_data in pairs(result.milter.add_headers) do
        kumo.log_info(
          string.format('Milter: Adding header %s: %s', header_name, header_data.value)
        )
      end
    end
    if result.milter.remove_headers then
      for header_name, _ in pairs(result.milter.remove_headers) do
        kumo.log_info(string.format('Milter: Removing header %s', header_name))
      end
    end
  end

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 7: Body Rewriting

Request and apply full message body rewriting from Rspamd.
This is useful when Rspamd modules make modifications beyond just headers
(e.g., stripping attachments, adding disclaimers, modifying MIME parts).

Note: Requires rspamd-client 0.4.0+
]]

--[[
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  use_file_path = true,
  zstd = true,
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  if conn_meta:get_meta 'authn_id' then
    return
  end

  -- Scan with body_block enabled to receive rewritten message
  local result = rspamd.scan_message(client, msg, conn_meta, {
    use_file_path = true,
    body_block = true,  -- Request rewritten body if Rspamd modifies it
  })

  -- Check if body was rewritten
  if result.rewritten_body then
    kumo.log_info(
      string.format(
        'Rspamd rewrote message body (%d bytes)',
        #result.rewritten_body
      )
    )
    -- Apply the rewritten body
    rspamd.apply_body_rewrite(msg, result)
  else
    -- No body modifications, just apply headers
    rspamd.apply_milter_actions(msg, result)
  end

  -- Apply action recommendations
  if result.action == 'reject' then
    kumo.reject(550, '5.7.1 Message rejected as spam')
  elseif result.score > 5 then
    msg:prepend_header('X-Spam-Flag', 'YES')
  end

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 8: Automatic Body Rewriting with Setup

The setup() helper automatically handles body rewriting when configured:
]]

--[[
rspamd:setup {
  base_url = 'http://localhost:11333',
  use_file_path = true,
  body_block = true,  -- Enable body rewriting support
  action = 'tag',  -- Tag spam, don't reject
  zstd = true,
}

kumo.on('smtp_server_message_received', rspamd.scan)
-- Body rewriting is applied automatically if Rspamd returns modified message
]]

--[[
# Example 9: Advanced Setup with HTTPCrypt Encryption

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
    msg:prepend_header('X-Greylist', 'YES')
    msg:prepend_header('X-Spam-Score', tostring(result.score))
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
# Example 10: Direct API Usage (Low-Level)

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
    -- Optional: request body rewriting
    body_block = true,
  }

  -- Get message data (not transmitted if file_path is set)
  local message_data = msg:get_data()

  -- Scan with compression and optional encryption
  -- If file_path is set, Rspamd reads from disk instead
  -- If body_block is set, rewritten body will be in result.rewritten_body
  local result = client:scan(message_data, metadata)

  -- Process results as needed
  print('Score:', result.score)
  print('Action:', result.action)
  for symbol, data in pairs(result.symbols or {}) do
    print('Symbol:', symbol, 'Score:', data.score or 0)
  end

  -- Apply body rewriting if present
  if result.rewritten_body then
    rspamd.apply_body_rewrite(msg, result)
  else
    -- Manually apply milter actions if needed
    if result.milter then
      rspamd.apply_milter_actions(msg, result)
    end
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
