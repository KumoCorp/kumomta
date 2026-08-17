-- A minimal maildir sink that presents a specific TLS certificate on its
-- ESMTP listener, for use by the DANE integration tests. The certificate and
-- key are supplied via env so the test can compute the matching TLSA record.
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  -- Keep accounting state isolated to this test; the defaults live in a shared,
  -- possibly non-writable location.
  kumo.configure_accounting_db_path ':memory:'
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
    tls_certificate = {
      key_data = os.getenv 'KUMOD_SINK_TLS_CERT',
    },
    tls_private_key = {
      key_data = os.getenv 'KUMOD_SINK_TLS_KEY',
    },
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

kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', 'maildir')
end)

kumo.on('get_queue_config', function(_domain, _tenant, _campaign)
  return kumo.make_queue_config {
    protocol = {
      maildir_path = TEST_DIR .. '/maildir',
    },
  }
end)
