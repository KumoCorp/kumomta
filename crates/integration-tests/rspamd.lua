-- Rspamd integration test policy
-- Uses recipient address patterns to determine test behavior
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local RSPAMD_URL = os.getenv 'KUMOD_TEST_RSPAMD_URL'

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:0',
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

-- Determine per-recipient spam threshold
local function get_spam_threshold_for_user(recipient)
  if recipient:match '@vip%.example%.com$' then
    return 100.0 -- Very lenient for VIP (won't reject normal messages)
  elseif recipient:match '@normal%.example%.com$' then
    return 5.0 -- Strict for normal users
  else
    return nil -- No threshold check
  end
end

-- Scan once per batch in smtp_server_data
kumo.on('smtp_server_data', function(msg)
  local config = {
    base_url = RSPAMD_URL,
    add_headers = true, -- Add headers in Rust before message is spooled
    reject_spam = false,
  }

  local result = kumo.rspamd.scan_message(config, msg)

  -- Store results in metadata for later use
  msg:set_meta('rspamd_score', result.score)
  msg:set_meta('rspamd_action', result.action)
end)

-- Apply per-recipient logic in smtp_server_message_received
kumo.on('smtp_server_message_received', function(msg)
  local score = msg:get_meta 'rspamd_score'
  if not score then
    return
  end

  local recipient = tostring(msg:recipient())
  local localpart = msg:recipient().user

  -- Test scenario: reject-spam@* - reject if spam action
  if localpart == 'reject-spam' then
    local action = msg:get_meta 'rspamd_action'
    if action == 'reject' or action == 'rewrite subject' or action == 'add header' then
      kumo.reject(
        550,
        string.format('5.7.1 Message rejected as spam (score: %.2f, action: %s)', score, action)
      )
    end
  end

  -- Test scenario: threshold-* - per-recipient threshold check
  local threshold = get_spam_threshold_for_user(recipient)
  if threshold then
    if score > threshold then
      kumo.reject(
        550,
        string.format(
          '5.7.1 Message rejected as spam (score: %.2f, threshold: %.2f)',
          score,
          threshold
        )
      )
    end

    msg:set_meta('spam_threshold', threshold)
  end

  -- For scan@* and headers@* - just accept and deliver
  msg:set_meta('queue', 'maildir')
end)

kumo.on('get_queue_config', function(_domain, _tenant, _campaign)
  return kumo.make_queue_config {
    protocol = {
      maildir_path = TEST_DIR .. '/maildir',
    },
  }
end)
