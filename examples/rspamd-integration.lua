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

--[[
# Example 1: Simplified API with Built-in Actions

The kumo.rspamd.scan_message() function provides a streamlined Rust-based interface
that automatically extracts message metadata and applies default actions.

Note: Use smtp_server_data for efficient per-batch scanning. This scans once
per SMTP transaction instead of once per recipient, avoiding duplicate scans.
]]

kumo.on('smtp_server_data', function(msg)
  -- Simple config with default actions
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,     -- Add X-Spam-* headers (default: true)
    reject_spam = true,     -- Reject spam messages (default: false)
    reject_soft = false,    -- Use 4xx instead of 5xx (default: false)
    prefix_subject = false, -- Prefix subject with ***SPAM*** (default: false)
  }

  -- Scan and apply default actions (all done in Rust)
  -- Scans once per batch (all recipients in this SMTP transaction)
  local result = kumo.rspamd.scan_message(config, msg)

  -- Result contains the full Rspamd response
  -- Default actions have already been applied based on config
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)

--[[
# Example 1a: Simplified API with Secure Credentials

Using HTTPCrypt encryption and secure credential storage with KeySource.

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

Sensitive fields (password, encryption_key, proxy_username, proxy_password)
support KeySource for secure storage:
- Inline string: 'my-secret-key'
- File: {path = '/path/to/secret'}
- Data: {key_data = 'inline-secret', format = 'text'}
]]

--[[
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = 'http://localhost:11333',

    -- Encryption key from file (recommended for production)
    encryption_key = { path = '/etc/kumo/rspamd_key.txt' },

    -- Or inline for simple deployments
    -- encryption_key = { key_data = os.getenv('RSPAMD_ENCRYPTION_KEY'), format = 'text' },

    -- Password also supports KeySource
    password = { path = '/etc/kumo/rspamd_password.txt' },

    add_headers = true,
    reject_spam = true,
  }

  -- Scans once per SMTP transaction (batch)
  local result = kumo.rspamd.scan_message(config, msg)

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 2: Custom Action Handling with Score-Based Logic

Use the Rust-based scan_message with custom Lua logic for score-based decisions.
The Rust implementation handles the Rspamd communication and applies configured
actions automatically. You can also add custom logic on top.
]]

--[[
kumo.on('smtp_server_data', function(msg)
  -- Configure Rspamd client to add headers only (no automatic rejection)
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,  -- Handle rejection logic in Lua
  }

  -- Scan the message (Rust handles communication and header addition)
  local result = kumo.rspamd.scan_message(config, msg)

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
    -- Probable spam - route to quarantine
    msg:set_meta('queue', 'quarantine')
  end

  -- Store results for logging
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
# Example 3: Subject Prefix for Spam

Enable subject prefixing to mark spam messages with ***SPAM*** prefix.
The Rust implementation handles this automatically when configured.
]]

--[[
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,
    prefix_subject = true,  -- Prefix subject with ***SPAM*** for spam
  }

  local result = kumo.rspamd.scan_message(config, msg)

  -- Subject prefixing is applied automatically by Rust for spam messages
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 4: Custom Rejection Messages

Customize rejection messages based on Rspamd results.
The Rust implementation handles the core functionality, and you can
add custom logic for rejection messages.
]]

--[[
kumo.on('smtp_server_data', function(msg)
  -- Let Rust handle headers, but not automatic rejection
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,  -- Handle rejection in Lua with custom message
  }

  local result = kumo.rspamd.scan_message(config, msg)

  -- Custom rejection logic with detailed message
  if result.action == 'reject' then
    local symbols = {}
    for name, _ in pairs(result.symbols or {}) do
      table.insert(symbols, name)
    end

    kumo.reject(
      550,
      string.format(
        '5.7.1 Message rejected due to spam characteristics. Score: %.2f. Symbols: %s',
        result.score,
        table.concat(symbols, ', ')
      )
    )
  elseif result.action == 'soft reject' then
    -- Custom handling for soft reject (greylist)
    kumo.reject(451, string.format('4.7.1 Greylisted. Score: %.2f', result.score))
  end

  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
]]

--[[
# Example 5: Per-Recipient Spam Thresholds

For per-recipient spam policies, scan once in smtp_server_data but apply
recipient-specific thresholds in smtp_server_message_received.

This pattern provides:
- Efficient scanning (once per SMTP transaction)
- Per-recipient policy decisions
- Ability to reject before final message acceptance
- Access to recipient-specific settings (quotas, preferences, etc.)
]]

--[[
-- Example function to get per-user spam threshold
-- In production, this would query a database or lookup service
local function get_spam_threshold_for_user(recipient)
  -- Example thresholds by domain
  if recipient.domain == 'vip.example.com' then
    return 15.0 -- VIP users: lenient threshold
  elseif recipient.domain == 'staff.example.com' then
    return 10.0 -- Staff: moderate threshold
  else
    return 5.0 -- Default: strict threshold
  end
end

-- Scan once per batch in smtp_server_data
kumo.on('smtp_server_data', function(msg)
  -- Configure to add headers but not reject automatically
  local config = {
    base_url = 'http://localhost:11333',
    add_headers = true,
    reject_spam = false,  -- Handle rejection per-recipient
  }

  -- Scan once for all recipients in this SMTP transaction
  local result = kumo.rspamd.scan_message(config, msg)

  -- Store results in metadata for later use
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  -- Log scan results
  kumo.log_info(
    string.format(
      'Rspamd scan: score=%.2f action=%s recipients=%d',
      result.score,
      result.action,
      #msg:recipient_list()
    )
  )
end)

-- Apply per-recipient thresholds in smtp_server_message_received
kumo.on('smtp_server_message_received', function(msg)
  local score = msg:get_meta 'rspamd_score'
  if not score then
    return -- No scan results available
  end

  -- Get recipient-specific threshold
  local recipient = msg:recipient()
  local threshold = get_spam_threshold_for_user(recipient)

  kumo.log_info(
    string.format(
      'Applying spam policy for %s: score=%.2f threshold=%.2f',
      tostring(recipient),
      score,
      threshold
    )
  )

  -- Apply recipient-specific policy
  if score > threshold then
    kumo.reject(
      550,
      string.format(
        '5.7.1 Message rejected as spam (score: %.2f, threshold: %.2f)',
        score,
        threshold
      )
    )
  elseif score > threshold * 0.7 then
    -- Close to threshold: route to junk folder
    msg:set_meta('queue', 'suspect')
  end

  -- Store per-recipient decision for logging
  msg:set_meta('spam_threshold', threshold)
  msg:set_meta('threshold_exceeded', score > threshold)
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
