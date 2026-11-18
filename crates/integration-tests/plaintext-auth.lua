local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local allow_plaintext = os.getenv 'KUMOD_ALLOW_PLAINTEXT_AUTH' == 'true'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
    allow_plaintext_auth = allow_plaintext,
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

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return authc == 'testuser' and password == 'testpass'
end)

kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', 'null')
end)
