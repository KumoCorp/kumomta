local kumo = require 'kumo'
-- This config acts as a sink that will discard all received mail

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
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

kumo.on('smtp_server_message_received', function(msg)
  -- Redirect traffic to the sink
  msg:set_meta('queue', 'localhost.')
end)

kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {}
end)

kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  -- Allow sending to a sink
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    smtp_port = SINK_PORT,
    prohibited_hosts = {},
  }
end)
