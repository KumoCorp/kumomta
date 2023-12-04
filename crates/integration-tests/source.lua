local kumo = require 'kumo'
-- This config acts as a sink that will discard all received mail

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local WEBHOOK_PORT = os.getenv 'KUMOD_WEBHOOK_PORT'

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
    headers = { 'X-*', 'Y-*' },
  }

  if WEBHOOK_PORT then
    kumo.configure_log_hook {
      name = 'webhook',
      headers = { 'Subject', 'X-*' },
    }
  end

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

if WEBHOOK_PORT then
  kumo.on('should_enqueue_log_record', function(msg)
    local log_record = msg:get_meta 'log_record'
    -- avoid an infinite loop caused by logging that we logged
    if log_record.queue ~= 'webhook' then
      msg:set_meta('queue', 'webhook')
      return true
    end
    return false
  end)

  kumo.on('make.webhook', function(_domain, _tenant, _campaign)
    local sender = {}
    local client = kumo.http.build_client {}
    function sender:send(message)
      local response = client
        :post(string.format('http://127.0.0.1:%d/log', WEBHOOK_PORT))
        :header('Content-Type', 'application/json')
        :body(message:get_data())
        :send()

      local disposition = string.format(
        '%d %s: %s',
        response:status_code(),
        response:status_reason(),
        response:text()
      )

      if response:status_is_success() then
        return disposition
      end
      kumo.reject(500, disposition)
    end
    return sender
  end)
end

kumo.on('smtp_server_message_received', function(msg) end)

kumo.on('get_queue_config', function(domain, _tenant, _campaign)
  if domain == 'webhook' then
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          constructor = 'make.webhook',
        },
      },
    }
  end
  return kumo.make_queue_config {
    protocol = {
      -- Redirect traffic to the sink
      smtp = {
        mx_list = { 'localhost' },
      },
    },
  }
end)

kumo.on('get_egress_path_config', function(_domain, _source_name, _site_name)
  -- Allow sending to a sink
  local params = {
    enable_tls = os.getenv 'KUMOD_ENABLE_TLS' or 'OpportunisticInsecure',
    smtp_port = SINK_PORT,
    prohibited_hosts = {},
  }

  local username = os.getenv 'KUMOD_SMTP_AUTH_USERNAME'
  local password = os.getenv 'KUMOD_SMTP_AUTH_PASSWORD'

  if username and password then
    params.smtp_auth_plain_username = username
    params.smtp_auth_plain_password = {
      key_data = password,
    }
  end

  return kumo.make_egress_path(params)
end)
