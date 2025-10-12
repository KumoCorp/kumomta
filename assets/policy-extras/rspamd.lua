local mod = {}
local kumo = require 'kumo'

--[[
# Rspamd Integration for KumoMTA

This module provides native Rspamd integration for spam scanning using the official
rspamd-client Rust library.

## Features

- Native async Rust client for high performance
- **File header optimization** for local scanning (avoids sending message body over socket)
- **Body rewriting** support for message modifications (requires rspamd-client 0.4+)
- **Milter protocol support** for header modifications (add/remove headers, subject rewriting)
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
  action = 'reject',  -- or 'tag', 'none', 'quarantine'
  use_file_path = true,  -- Use File header for local scanning (recommended)
  zstd = true,  -- Enable compression (default: true)
  encryption_key = os.getenv('RSPAMD_KEY'),  -- Optional HTTPCrypt encryption
}

-- Use smtp_server_data for per-batch scanning (recommended)
kumo.on('smtp_server_data', rspamd.scan)
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

-- Use smtp_server_data for per-batch scanning
kumo.on('smtp_server_data', function(msg, conn_meta)
  -- Skip scanning for authenticated users
  if conn_meta:get_meta('authn_id') then
    return
  end

  -- Scan the message with file path optimization
  -- This scans once per SMTP transaction (batch), not once per recipient
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

## Per-Recipient Spam Thresholds

For per-recipient spam policies, scan once in `smtp_server_data` but apply
recipient-specific thresholds in `smtp_server_message_received`:

```lua
local rspamd = require 'policy-extras.rspamd'
local client = kumo.rspamd.build_client {
  base_url = 'http://localhost:11333',
  zstd = true,
}

-- Scan once per batch in smtp_server_data
kumo.on('smtp_server_data', function(msg, conn_meta)
  local result = rspamd.scan_message(client, msg, conn_meta, {use_file_path = true})

  -- Store results in metadata for later use
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)

  -- Add headers but don't reject yet
  rspamd.apply_milter_actions(msg, result)
end)

-- Apply per-recipient thresholds in smtp_server_message_received
kumo.on('smtp_server_message_received', function(msg)
  local score = msg:get_meta('rspamd_score')
  if not score then
    return -- No scan results available
  end

  -- Get recipient-specific threshold
  local recipient = tostring(msg:recipient())
  local threshold = get_spam_threshold_for_user(recipient) -- Your custom function

  if score > threshold then
    kumo.reject(550, string.format(
      '5.7.1 Message rejected as spam (score: %.2f, threshold: %.2f)',
      score, threshold
    ))
  end
end)
```

This pattern provides:
- Efficient scanning (once per SMTP transaction)
- Per-recipient policy decisions
- Ability to reject before final message acceptance
- Access to recipient-specific settings (quotas, preferences, etc.)

]]

-- Valid Rspamd actions (as returned by Rspamd server)
local VALID_RSPAMD_ACTIONS = {
  ['no action'] = true,
  ['add header'] = true,
  ['rewrite subject'] = true,
  ['soft reject'] = true,
  greylist = true,
  reject = true,
}

-- Valid module action configurations (for setup() config.action parameter)
local VALID_MODULE_ACTIONS = {
  none = true, -- Don't reject, just store metadata and apply modifications
  tag = true, -- Add headers but never reject
  reject = true, -- Follow Rspamd's recommendation for rejections
  quarantine = true, -- Send spam to quarantine queue
}

-- Helper function to get keys from a table
local function table_keys(t)
  local keys = {}
  for k, _ in pairs(t) do
    table.insert(keys, k)
  end
  table.sort(keys)
  return keys
end

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
    -- Subject rewriting is typically handled by milter actions
    -- But we can also do it manually if milter doesn't provide it
    local subject = msg:get_first_named_header_value 'Subject' or ''
    if not result.milter or not result.milter.add_headers or not result.milter.add_headers.Subject then
      msg:remove_all_named_headers('Subject')
      msg:prepend_header('Subject', '***SPAM*** ' .. subject)
    end
    msg:prepend_header('X-Spam-Flag', 'YES')
    msg:prepend_header('X-Spam-Score', tostring(result.score))
  end,

  ['add header'] = function(msg, result)
    msg:prepend_header('X-Spam-Flag', 'YES')
    msg:prepend_header('X-Spam-Score', tostring(result.score))

    -- Add symbol list
    local symbols = {}
    for name, _ in pairs(result.symbols or {}) do
      table.insert(symbols, name)
    end
    if #symbols > 0 then
      msg:prepend_header('X-Spam-Symbols', table.concat(symbols, ', '))
    end
  end,

  ['no action'] = function(msg, result)
    msg:prepend_header('X-Spam-Flag', 'NO')
    msg:prepend_header('X-Spam-Score', tostring(result.score))
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
--   body_block: if true, request rewritten body in response (for body modifications)
--
-- Note: This function works with both smtp_server_data (batch scanning) and
-- smtp_server_message_received (per-message scanning) events.
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

    -- Get recipients - handles both single and multiple recipients
    -- In smtp_server_data, this returns all recipients in the batch
    -- In smtp_server_message_received, this returns a single recipient
    local recipients = {}
    local recip_list = msg:recipient_list()
    if recip_list then
      for _, recip in ipairs(recip_list) do
        table.insert(recipients, tostring(recip))
      end
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

    -- Request rewritten body if modifications are expected
    -- This enables Rspamd to return the modified message body
    if options.body_block then
      metadata.body_block = true
    end
  end

  return metadata
end

-- Scan a message using the Rspamd client
-- Options:
--   use_file_path: if true, use File header for local scanning (recommended for localhost)
--   body_block: if true, request rewritten body in response
--
-- Best Practice: Use this in smtp_server_data for per-batch scanning to avoid
-- duplicate scans when a message has multiple recipients. The scan result should
-- be stored in message metadata for use in later events.
function mod.scan_message(client, msg, conn_meta, options)
  options = options or {}
  local metadata = mod.extract_metadata(msg, conn_meta, options)

  -- Get message data (may not be transmitted if using file_path)
  local message_data = msg:get_data()

  -- Scan with metadata
  -- If file_path is set, Rspamd will read from disk instead of using message_data
  -- If body_block is set, rewritten body will be in result.rewritten_body
  local result = client:scan(message_data, metadata)

  return result
end

-- Apply body rewriting from Rspamd response
-- Replaces the entire message body if Rspamd returned a rewritten version
function mod.apply_body_rewrite(msg, result)
  if not result.rewritten_body then
    return false
  end

  -- Replace entire message with rewritten version
  msg:set_data(result.rewritten_body)
  return true
end

-- Apply milter actions from Rspamd response
-- Handles add_headers, remove_headers, and subject rewriting
function mod.apply_milter_actions(msg, result)
  if not result.milter then
    return
  end

  local milter = result.milter

  -- Remove headers
  if milter.remove_headers then
    for header_name, index in pairs(milter.remove_headers) do
      -- If index is 0, remove all instances
      if index == 0 then
        msg:remove_all_named_headers(header_name)
      else
        -- Remove specific instance (1-indexed)
        -- Note: KumoMTA may handle this differently
        msg:remove_all_named_headers(header_name)
      end
    end
  end

  -- Add headers
  if milter.add_headers then
    for header_name, header_data in pairs(milter.add_headers) do
      -- Special handling for Subject rewriting
      if header_name == 'Subject' then
        msg:remove_all_named_headers('Subject')
        msg:prepend_header('Subject', header_data.value)
      else
        -- Add the header
        -- The order field could be used for insertion ordering if needed
        msg:append_header(header_name, header_data.value)
      end
    end
  end
end

-- Apply the Rspamd action recommendation
-- Options:
--   apply_milter: if true, apply milter actions (default: true)
--   apply_body: if true, apply body rewriting (default: true)
function mod.apply_action(msg, result, custom_handlers, options)
  options = options or {}
  local apply_milter = options.apply_milter ~= false -- default true
  local apply_body = options.apply_body ~= false -- default true

  local handlers = custom_handlers or ACTION_HANDLERS
  local action = result.action

  -- Apply body rewriting first (if present)
  -- This replaces the entire message including headers
  if apply_body and mod.apply_body_rewrite(msg, result) then
    kumo.log_info 'Applied body rewrite from Rspamd'
    -- Body was rewritten, milter actions are already applied in the rewritten body
    return
  end

  local handler = handlers[action]
  if handler then
    handler(msg, result)
  else
    -- Unknown action: this is an error condition
    if not VALID_RSPAMD_ACTIONS[action] then
      error(
        string.format(
          "Unknown Rspamd action '%s' received from server. Valid actions are: %s",
          action,
          table.concat(table_keys(VALID_RSPAMD_ACTIONS), ', ')
        )
      )
    end
    -- Valid Rspamd action but no handler defined - add headers as fallback
    kumo.log_warn(
      string.format(
        "No handler defined for Rspamd action '%s', adding headers only",
        action
      )
    )
    msg:prepend_header('X-Spam-Status', action)
    msg:prepend_header('X-Spam-Score', tostring(result.score))
  end

  -- Apply milter actions (headers, subject rewrite, etc.)
  -- Only if body wasn't rewritten (milter actions are in the rewritten body)
  if apply_milter then
    mod.apply_milter_actions(msg, result)
  end
end

-- Simple setup helper
function mod:setup(config)
  if mod.CONFIGURED then
    error 'rspamd module has already been configured'
  end

  config = config or {}

  -- Validate action parameter if specified
  if config.action and not VALID_MODULE_ACTIONS[config.action] then
    error(
      string.format(
        "Invalid action '%s' in rspamd configuration. Valid actions are: %s",
        config.action,
        table.concat(table_keys(VALID_MODULE_ACTIONS), ', ')
      )
    )
  end

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
    body_block = config.body_block, -- Request rewritten body if modifications occur
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
      -- Just store metadata and apply modifications
      mod.apply_body_rewrite(msg, result)
      mod.apply_milter_actions(msg, result)
      return
    elseif config.action == 'tag' then
      -- Apply body rewrite if present
      if not mod.apply_body_rewrite(msg, result) then
        -- Only add headers if body wasn't rewritten
        ACTION_HANDLERS['add header'](msg, result)
        mod.apply_milter_actions(msg, result)
      end
    elseif config.action == 'reject' then
      -- Reject if Rspamd says to reject
      if result.action == 'reject' or result.action == 'soft reject' or result.action == 'greylist' then
        mod.apply_action(msg, result, config.custom_handlers)
      else
        if not mod.apply_body_rewrite(msg, result) then
          ACTION_HANDLERS['add header'](msg, result)
          mod.apply_milter_actions(msg, result)
        end
      end
    elseif config.action == 'quarantine' then
      -- Send spam to quarantine queue
      if result.action == 'reject' or result.score > (config.quarantine_threshold or 5) then
        msg:set_meta('queue', config.quarantine_queue or 'quarantine')
        if not mod.apply_body_rewrite(msg, result) then
          ACTION_HANDLERS['add header'](msg, result)
        end
      end
      if not mod.apply_body_rewrite(msg, result) then
        mod.apply_milter_actions(msg, result)
      end
    else
      -- Follow Rspamd's recommendation
      mod.apply_action(msg, result, config.custom_handlers)
    end
  end

  -- Store for external use
  mod.scan = scan_handler
  mod.client = client

  -- Store configuration for validation
  mod.CONFIGURED = {
    config = config,
    client_config = client_config,
  }

  return scan_handler
end

-- Validation hook for kumo --validate
kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local config = mod.CONFIGURED.config
  local failed = false

  local function show_error(message)
    if not failed then
      failed = true
      kumo.validation_failed()
      print 'Issues found in rspamd configuration:'
    end
    print(string.format('  - %s', message))
  end

  -- Validate action parameter
  if config.action and not VALID_MODULE_ACTIONS[config.action] then
    show_error(
      string.format(
        "Invalid action '%s'. Valid actions are: %s",
        config.action,
        table.concat(table_keys(VALID_MODULE_ACTIONS), ', ')
      )
    )
  end

  -- Validate base_url if it's a string (it should be, but check anyway)
  if config.base_url and type(config.base_url) == 'string' then
    if not config.base_url:match '^https?://' then
      show_error(
        string.format(
          "base_url '%s' should start with 'http://' or 'https://'",
          config.base_url
        )
      )
    end
  end

  -- Validate quarantine settings
  if config.action == 'quarantine' then
    if
      config.quarantine_threshold
      and type(config.quarantine_threshold) ~= 'number'
    then
      show_error 'quarantine_threshold must be a number'
    end
    if
      config.quarantine_queue
      and type(config.quarantine_queue) ~= 'string'
    then
      show_error 'quarantine_queue must be a string'
    end
  end

  -- Validate custom_handlers if present
  if config.custom_handlers then
    if type(config.custom_handlers) ~= 'table' then
      show_error 'custom_handlers must be a table'
    else
      for action, handler in pairs(config.custom_handlers) do
        if not VALID_RSPAMD_ACTIONS[action] then
          show_error(
            string.format(
              "custom_handlers contains unknown action '%s'. Valid actions are: %s",
              action,
              table.concat(table_keys(VALID_RSPAMD_ACTIONS), ', ')
            )
          )
        end
        if type(handler) ~= 'function' then
          show_error(
            string.format(
              "custom_handlers['%s'] must be a function, got %s",
              action,
              type(handler)
            )
          )
        end
      end
    end
  end
end)

return mod
