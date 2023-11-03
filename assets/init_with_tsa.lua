local kumo = require 'kumo'
local shaping = require 'policy-extras.shaping'

local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = { 'assets/policy-extras/shaping.toml' },
}

kumo.on('init', function()
  kumo.configure_accounting_db_path '/tmp/acct.db'

  -- Configure publishing of logs to automation daemon
  shaper.setup_publish()

  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2025',
  }

  kumo.configure_local_logs {
    log_dir = '/var/tmp/kumo-logs',
    max_segment_duration = '1s',
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }
  kumo.start_http_listener {
    use_tls = true,
    listen = '0.0.0.0:8001',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
    kind = 'RocksDB',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
    kind = 'RocksDB',
  }
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
  }
end)

kumo.on('get_egress_path_config', shaper.get_egress_path_config)
