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

-- Custom Lua dispatcher whose send method sleeps long enough for
-- the inspect/abort integration test to observe and act on it, but
-- with the watchdog turned off so the test, not the watchdog, drives
-- the abort.
kumo.on('make.slow_send', function(_domain, _tenant, _campaign)
  local sender = {}
  function sender:send(_message)
    kumo.sleep(60)
  end
  return sender
end)

kumo.on(
  'get_queue_config',
  function(domain, _tenant, _campaign, _routing_domain)
    local cfg = {
      protocol = {
        custom_lua = {
          constructor = 'make.slow_send',
        },
      },
    }
    -- For the scheduled-queue-constraint integration test.
    if domain == 'sched-rate.example.com' then
      cfg.max_message_rate = '100/s'
    end
    return kumo.make_queue_config(cfg)
  end
)

kumo.on('get_egress_path_config', function(domain, _source_name, _site_name)
  local cfg = {
    connection_limit = 1,
    -- A large value so the watchdog cannot interfere; the test
    -- aborts the dispatcher itself via the admin API.
    dispatcher_progress_watchdog_timeout = '1h',
  }
  -- For the scheduled-queue-constraint integration test: the path
  -- declares a max_message_rate of 1000/s, which the scheduled
  -- queue's 100/s rate then shadows.
  if domain == 'sched-rate.example.com' then
    cfg.max_message_rate = '1000/s'
  end
  return kumo.make_egress_path(cfg)
end)
