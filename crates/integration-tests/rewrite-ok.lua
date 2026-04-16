local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  local relay_hosts = { '0.0.0.0/0' }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
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

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    protocol = {
      -- Redirect traffic to the sink
      smtp = {
        mx_list = { 'localhost:' .. SINK_PORT },
      },
    },
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  local params = {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
  }
  return kumo.make_egress_path(params)
end)

kumo.on(
  'smtp_server_rewrite_response',
  function(status, response, command, conn_meta)
    if command == 'DATA' and status == 250 then
      return 250, 'super fantastic!\n' .. response
    end
    kumo.log_info('doing nothing with', status, message, command)
    -- implicitly testing the nil return case for everything other than DATA
  end
)
