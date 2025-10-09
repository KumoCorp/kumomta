local mod = {}
local kumo = require 'kumo'

--[[
# Rspamd Integration for KumoMTA

This module provides native Rspamd integration for spam scanning using the official
rspamd-client Rust library.

## Features

- Native async Rust client for high performance
- **File header optimization** for local scanning (avoids sending message body over socket)
- **ZSTD compression** for faster message transmission (enabled by default)
- **HTTPCrypt encryption** for secure communication with Rspamd
- Automatic metadata extraction from messages and connections
- Flexible action handling (reject, tag, quarantine, etc.)
- Connection pooling and error handling
- Proxy support for remote Rspamd instances
- TLS/mTLS support

## Basic Usage

```lua
local rspamd = require 'policy-extras.rspamd'

rspamd:setup {
  base_url = 'http://localhost:11333',
  action = 'reject',  -- or 'tag', 'discard', 'quarantine'
  use_file_path = true,  -- Use File header for local scanning (recommended)
  zstd = true,  -- Enable compression (default: true)
  encryption_key = os.getenv('RSPAMD_KEY'),  -- Optional HTTPCrypt encryption
}

kumo.on('smtp_server_message_received', rspamd.scan)
```

## Advanced Usage

```lua
local rspamd = require 'policy-extras.rspamd'

-- Build a custom client with HTTPCrypt encryption
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  timeout = '10s',
  password = os.getenv('RSPAMD_PASSWORD'),
  zstd = true,  -- Compress message bodies (default: true)
  encryption_key = os.getenv('RSPAMD_ENCRYPTION_KEY'),  -- HTTPCrypt encryption
  retries = 2,  -- Retry failed requests
}

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Skip scanning for authenticated users
  if conn_meta:get_meta('authn_id') then
    return
  end

  -- Scan the message with file path optimization
  local result = rspamd.scan_message(client, msg, conn_meta, {use_file_path = true})

  -- Custom action handling
  if result.score > 15 then
    kumo.reject(550, '5.7.1 Message rejected as spam')
  elseif result.score > 5 then
    msg:set_first_named_header('X-Spam-Flag', 'YES')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end

  -- Store for logging
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)
```

]]

-- Default action handlers
local ACTION_HANDLERS = {
  reject = function(msg, result)
    kumo.reject(
      550,
      string.format('5.7.1 Spam detected (score: %.2f)', result.score)
    )
  end,

  ['soft reject'] = function(msg, result)
    kumo.reject(
      451,
      string.format('4.7.1 Greylisted, please try again (score: %.2f)', result.score)
    )
  end,

  ['rewrite subject'] = function(msg, result)
    local subject = msg:get_first_named_header_value 'Subject' or ''
    msg:set_first_named_header('Subject', '***SPAM*** ' .. subject)
    msg:set_first_named_header('X-Spam-Flag', 'YES')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end,

  ['add header'] = function(msg, result)
    msg:set_first_named_header('X-Spam-Flag', 'YES')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))

    -- Add symbol list
    local symbols = {}
    for name, _ in pairs(result.symbols or {}) do
      table.insert(symbols, name)
    end
    if #symbols > 0 then
      msg:set_first_named_header('X-Spam-Symbols', table.concat(symbols, ', '))
    end
  end,

  ['no action'] = function(msg, result)
    msg:set_first_named_header('X-Spam-Flag', 'NO')
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end,

  greylist = function(msg, result)
    kumo.reject(
      451,
      string.format('4.7.1 Greylisted, please try again (score: %.2f)', result.score)
    )
  end,
}

-- Extract metadata from connection and message for Rspamd headers
-- Options:
--   use_file_path: if true and Rspamd is on localhost/unix socket, use File header
--                  to avoid sending message body (significant performance improvement)
function mod.extract_metadata(msg, conn_meta, options)
  options = options or {}
  local metadata = {}

  -- Get connection metadata
  if conn_meta then
    metadata.ip = conn_meta:get_meta 'received_from'
    metadata.helo = conn_meta:get_meta 'ehlo_domain'
    metadata.user = conn_meta:get_meta 'authn_id'
    metadata.hostname = conn_meta:get_meta 'hostname'
  end

  -- Get message metadata
  if msg then
    metadata.from = tostring(msg:sender())
    metadata.queue_id = msg:id()

    -- Get recipients
    local recipients = {}
    local recip = msg:recipient()
    if recip then
      table.insert(recipients, tostring(recip))
    end
    metadata.rcpt = recipients

    -- Optionally use file path for local scanning (File header optimization)
    -- This avoids sending the message body over the socket
    -- Only use this if Rspamd is on localhost or unix socket
    if options.use_file_path then
      local spool_path = msg:get_meta 'spool_path'
      if spool_path then
        metadata.file_path = spool_path
      end
    end
  end

  return metadata
end

-- Scan a message using the Rspamd client
-- Options:
--   use_file_path: if true, use File header for local scanning (recommended for localhost)
function mod.scan_message(client, msg, conn_meta, options)
  options = options or {}
  local metadata = mod.extract_metadata(msg, conn_meta, options)

  -- Get message data (may not be transmitted if using file_path)
  local message_data = msg:get_data()

  -- Scan with metadata
  -- If file_path is set, Rspamd will read from disk instead of using message_data
  local result = client:scan(message_data, metadata)

  return result
end

-- Apply the Rspamd action recommendation
function mod.apply_action(msg, result, custom_handlers)
  local handlers = custom_handlers or ACTION_HANDLERS
  local action = result.action

  local handler = handlers[action]
  if handler then
    handler(msg, result)
  else
    -- Default: add headers for unknown actions
    kumo.log_warn(
      string.format(
        "Unknown Rspamd action '%s', adding headers only",
        action
      )
    )
    msg:set_first_named_header('X-Spam-Status', action)
    msg:set_first_named_header('X-Spam-Score', tostring(result.score))
  end
end

-- Simple setup helper
function mod:setup(config)
  config = config or {}

  -- Build the client
  local client_config = {
    base_url = config.base_url or 'http://localhost:11333',
    timeout = config.timeout or '10s',
    password = config.password,
    zstd = config.zstd ~= false, -- default true (compression)
    encryption_key = config.encryption_key, -- optional HTTPCrypt key
    retries = config.retries,
    proxy_url = config.proxy_url,
    proxy_username = config.proxy_username,
    proxy_password = config.proxy_password,
  }

  local client = kumo.rspamd.build_client(client_config)

  -- Scan options
  local scan_options = {
    use_file_path = config.use_file_path, -- Use File header for local scanning
  }

  -- Create scan function
  local function scan_handler(msg, conn_meta)
    -- Skip if configured
    if config.skip_authenticated and conn_meta:get_meta 'authn_id' then
      return
    end

    -- Scan the message
    local result = mod.scan_message(client, msg, conn_meta, scan_options)

    -- Store metadata
    msg:set_meta('rspamd_score', result.score)
    msg:set_meta('rspamd_action', result.action)

    -- Apply action based on config
    if config.action == 'none' then
      -- Just store metadata, don't take action
      return
    elseif config.action == 'tag' then
      -- Only add headers
      ACTION_HANDLERS['add header'](msg, result)
    elseif config.action == 'reject' then
      -- Reject if Rspamd says to reject
      if result.action == 'reject' or result.action == 'soft reject' or result.action == 'greylist' then
        mod.apply_action(msg, result, config.custom_handlers)
      else
        ACTION_HANDLERS['add header'](msg, result)
      end
    elseif config.action == 'quarantine' then
      -- Send spam to quarantine queue
      if result.action == 'reject' or result.score > (config.quarantine_threshold or 5) then
        msg:set_meta('queue', config.quarantine_queue or 'quarantine')
        ACTION_HANDLERS['add header'](msg, result)
      end
    else
      -- Follow Rspamd's recommendation
      mod.apply_action(msg, result, config.custom_handlers)
    end
  end

  -- Store for external use
  mod.scan = scan_handler
  mod.client = client

  return scan_handler
end

return mod
