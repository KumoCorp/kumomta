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
  kumo.define_spool { name = 'data', path = TEST_DIR .. '/data-spool' }
  kumo.define_spool { name = 'meta', path = TEST_DIR .. '/meta-spool' }
  kumo.dns.set_mta_sts_enabled(false)
  kumo.dns.configure_test_resolver {
    '$ORIGIN route-a.example.\n@ 600 MX 10 a.x.targets.test.\n@ 600 MX 20 b.x.targets.test.\n@ 600 MX 30 c.y.targets.test.\n',
    '$ORIGIN route-b.example.\n@ 600 MX 5 c.y.targets.test.\n@ 600 MX 10 b.x.targets.test.\n@ 600 MX 20 a.x.targets.test.\n',
    '$ORIGIN route-c.example.\n@ 600 MX 10 a.x.targets.test.\n@ 600 MX 20 b.y.targets.test.\n@ 600 MX 30 c.x.targets.test.\n',
    '$ORIGIN targets.test.\na.x 600 A 127.0.0.1\nb.x 600 A 127.0.0.1\nc.y 600 A 127.0.0.1\nb.y 600 A 127.0.0.1\nc.x 600 A 127.0.0.1\n',
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
