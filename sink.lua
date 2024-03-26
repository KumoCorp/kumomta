-- This config acts as a sink that will discard all received mail
local kumo = require 'kumo'

kumo.on('init', function()
  kumo.configure_accounting_db_path(os.tmpname())
  -- Define a listener.
  -- Can be used multiple times with different parameters to
  -- define multiple listeners!
  for _, port in ipairs { 2026 } do
    kumo.start_esmtp_listener {
      listen = '0:' .. tostring(port),
      relay_hosts = { '0.0.0.0/0' },
    }
  end
  kumo.start_http_listener {
    listen = '0.0.0.0:8002',
    trusted_hosts = { '127.0.0.1', '::1', '192.168.1.0/24' },
  }

  -- Define the default "data" spool location.
  -- This is unused by this config, but we are required to
  -- define a default spool location.
  kumo.define_spool {
    name = 'data',
    path = '/tmp/kumo-sink/data',
  }

  -- Define the default "meta" spool location.
  -- This is unused by this config, but we are required to
  -- define a default spool location.
  kumo.define_spool {
    name = 'meta',
    path = '/tmp/kumo-sink/meta',
  }
end)

kumo.on('smtp_server_message_received', function(msg)
  -- Accept and discard all messages
  msg:set_meta('queue', 'null')
end)

kumo.on('http_message_generated', function(msg)
  -- Accept and discard all messages
  msg:set_meta('queue', 'null')
end)
