local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  -- Capture rcpt_meta into every log record so tests can assert on it.
  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
    meta = { 'rcpt_meta' },
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

kumo.on(
  'get_queue_config',
  function(domain, _tenant, _campaign, _routing_domain)
    return kumo.make_queue_config {
      protocol = {
        smtp = {
          mx_list = { 'localhost:' .. SINK_PORT },
        },
      },
    }
  end
)

kumo.on('get_egress_path_config', function(_domain, _source_name, _site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
  }
end)
