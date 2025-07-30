local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

local queue_module = require 'policy-extras.queue'
local log_hooks = require 'policy-extras.log_hooks'

local queue_helper = queue_module:setup {
  {
    queue = {
      default = {
        -- Redirect traffic to the sink
        protocol = {
          smtp = {
            mx_list = { 'localhost:' .. SINK_PORT },
          },
        },
      },
    },
  },
}

local function ndr_generator(msg, log_record)
  print('NDR?', kumo.serde.json_encode_pretty(log_record))

  local bounce_msg = kumo.generate_rfc3464_message({
    include_original_message = 'FullContent',
    enable_expiration = false,
    enable_bounce = true,
    reporting_mta = {
      mta_type = 'dns',
      name = 'mta1.example.com',
    },
    stable_content = true,
  }, msg, log_record)

  if bounce_msg then
    print 'YES, try injecting'
    local ok, err = pcall(kumo.inject_message, bounce_msg)
    if not ok then
      kumo.log_error('failed to inject NDR: ', err)
    end
  end
end

log_hooks:new_disposition_hook {
  name = 'ndr_generator',
  hook = ndr_generator,
}

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  local relay_hosts = { '0.0.0.0/0' }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
    deferred_queue = false,
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
    headers = { 'X-*', 'Y-*', 'Subject' },
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

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
    log_oob = true,
    log_arf = true,
  }
end)

kumo.on('smtp_server_message_received', function(msg)
  local result = msg:import_scheduling_header 'X-Schedule'
  kumo.log_info('schedule result', kumo.serde.json_encode(result))
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  -- Allow sending to a sink
  local params = {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    opportunistic_tls_reconnect_on_failed_handshake = false,
  }

  kumo.log_warn('get_egress_path_config *******************', domain)

  return kumo.make_egress_path(params)
end)
