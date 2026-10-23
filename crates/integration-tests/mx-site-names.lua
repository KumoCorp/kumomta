-- Exercise DNS-MX queue identity without TLS or external DNS dependencies.
local kumo = require 'kumo'
local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
  }
  kumo.start_http_listener { listen = '127.0.0.1:0' }
  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
  }
  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
    kind = 'RocksDB',
  }
  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
    kind = 'RocksDB',
  }
  kumo.dns.set_mta_sts_enabled(false)
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN route-a.example.
@ 600 MX 10 a.x.targets.test.
@ 600 MX 20 b.x.targets.test.
@ 600 MX 30 c.y.targets.test.
]],
    [[
$ORIGIN route-b.example.
@ 600 MX 5 c.y.targets.test.
@ 600 MX 10 b.x.targets.test.
@ 600 MX 20 a.x.targets.test.
]],
    [[
$ORIGIN route-c.example.
@ 600 MX 10 a.x.targets.test.
@ 600 MX 20 b.y.targets.test.
@ 600 MX 30 c.x.targets.test.
]],
    [[
$ORIGIN targets.test.
a.x 600 A 127.0.0.1
b.x 600 A 127.0.0.1
c.y 600 A 127.0.0.1
b.y 600 A 127.0.0.1
c.x 600 A 127.0.0.1
]],
  }
end)

kumo.on('get_queue_config', function()
  return kumo.make_queue_config { retry_interval = '1h' }
end)

kumo.on('get_egress_path_config', function(domain)
  return kumo.make_egress_path {
    enable_tls = 'Disabled',
    prohibited_hosts = {},
    smtp_port = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT'),
    connection_limit = 1,
    ehlo_domain = domain,
    idle_timeout = '1m',
  }
end)
