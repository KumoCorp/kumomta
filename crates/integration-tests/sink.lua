local kumo = require 'kumo'
-- This config acts as a sink that will discard all received mail

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
    enable_auth_login = (os.getenv 'KUMOD_ENABLE_AUTH_LOGIN') and true,
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
end)

kumo.on('smtp_server_message_received', function(msg)
  -- Accept and discard all messages
  msg:set_meta('queue', 'null')
end)

function simple_auth_check(user, password)
  local password_database = {
    ['daniel'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[user] == password
end

kumo.on('http_server_validate_auth_basic', function(user, password)
  return simple_auth_check(user, password)
end)

-- This event is also used for AUTH LOGIN; conn_meta tells them apart
kumo.on('smtp_server_auth_plain', function(authz, authc, password, conn_meta)
  local mechanism = conn_meta:get_meta 'auth_mechanism'
  print(
    string.format(
      "AUTH %s: authz='%s' authc='%s' pass='%s'",
      mechanism,
      authz,
      authc,
      password
    )
  )
  -- Lets the tests confirm the mechanism is reported
  if mechanism ~= 'PLAIN' and mechanism ~= 'LOGIN' then
    return false
  end
  return simple_auth_check(authc, password)
end)
