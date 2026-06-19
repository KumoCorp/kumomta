local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }

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
end)

kumo.on('get_listener_domain', function(_domain, _listener, _conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
  }
end)

-- Custom Lua dispatcher whose send method never returns. The
-- progress watchdog must catch this and abort the dispatcher.
kumo.on('make.wedge_send', function(_domain, _tenant, _campaign)
  local sender = {}
  function sender:send(_message)
    kumo.sleep(86400)
  end
  return sender
end)

kumo.on('get_queue_config', function(_domain, _tenant, _campaign, _routing_domain)
  return kumo.make_queue_config {
    protocol = {
      custom_lua = {
        constructor = 'make.wedge_send',
      },
    },
  }
end)

kumo.on('get_egress_path_config', function(_domain, _source_name, _site_name)
  return kumo.make_egress_path {
    connection_limit = 1,
    dispatcher_progress_watchdog_timeout = '2s',
  }
end)
