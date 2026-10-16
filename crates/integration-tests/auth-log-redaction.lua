local kumo = require 'kumo'
local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '127.0.0.1' },
  }
  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    meta = { 'response_command' },
  }
  for _, name in ipairs { 'data', 'meta' } do
    kumo.define_spool {
      kind = 'RocksDB',
      name = name,
      path = TEST_DIR .. '/' .. name .. '-spool',
    }
  end
end)

kumo.on('smtp_server_auth_plain', function(_authz, authc, _password)
  if authc == 'defer' then
    kumo.reject(454, '4.7.0 temporary authentication failure')
  end
  return false
end)

kumo.on(
  'smtp_server_rewrite_response',
  function(_status, _response, command, conn_meta)
    -- Capture exactly what policy sees, as well as the native response.command.
    conn_meta:set_meta('response_command', command)
  end
)
