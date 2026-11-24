local kumo = require 'kumo'

kumo.on('init', function()
  local dir = os.getenv 'KUMOD_TEST_DIR'
  kumo.configure_accounting_db_path ':memory:'
  kumo.define_spool { name = 'data', path = dir .. '/data', kind = 'RocksDB' }
  kumo.define_spool { name = 'meta', path = dir .. '/meta', kind = 'RocksDB' }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    meta = {
      x_route = 'bulk',
      x_customer = 'example',
      x_auth_context = 'private',
      legacy_key = 'unclassified',
    },
  }
  kumo.start_http_listener { listen = '127.0.0.1:0' }
  kumo.start_http_listener {
    listen = '127.0.0.1:0',
    smtp_trace_meta_detail = {
      inbound_connection = {
        default_for_x_keys = 'envelope',
        keys = { x_route = 'operational', x_auth_context = 'secrets' },
      },
    },
  }
end)
