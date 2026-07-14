-- Source policy for the idna_starttls integration test.
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

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
  }

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }

  -- The MX target is expressed as a U-label so that the resolved hostname
  -- handed to the SMTP/TLS layer is in its unicode form, reproducing the
  -- regression from https://github.com/KumoCorp/kumomta/issues/533.
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN mx-sink.wezfurlong.org.
xn--mnchen-3ya 600 MX 10 münchen.mx-sink.wezfurlong.org.
münchen                600 A 127.0.0.1
]],
  }
end)

kumo.on('get_egress_path_config', function()
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    smtp_port = SINK_PORT,
  }
end)
