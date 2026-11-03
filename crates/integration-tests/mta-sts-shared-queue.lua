-- Fixture for the MTA-STS cleartext-veto test: nopolicy.example.com and
-- enforce.example.com share one ready queue because both resolve to the
-- same MX host, shared.example.com.
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
    kind = 'RocksDB',
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    kind = 'RocksDB',
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }

  kumo.dns.configure_test_resolver {
    [[
$ORIGIN nopolicy.example.com.
@    600 MX 10 shared.example.com.
]],
    [[
$ORIGIN enforce.example.com.
@    600 MX 10 shared.example.com.
]],
    [[
$ORIGIN shared.example.com.
@    600 A  127.0.0.1
]],
  }

  kumo.dns.configure_test_mta_sts {
    ['enforce.example.com'] = [[
version: STSv1
mode: enforce
mx: shared.example.com
max_age: 86400
]],
  }
end)

kumo.on('get_queue_config', function(domain)
  -- Returning nil as the protocol makes kumod resolve real MX records for
  -- the domain instead of a stubbed route.
  return kumo.make_queue_config {
    protocol = nil,
    retry_interval = '2s',
  }
end)

kumo.on('get_egress_path_config', function(domain)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
    prohibited_hosts = {},
    -- There is no real MTA on port 25 for the resolved MX host.
    -- Redirect delivery to the local sink listener instead.
    smtp_port = SINK_PORT,
    enable_mta_sts = true,
  }
end)
