local kumo = require 'kumo'

kumo.on('proxy_init', function()
  kumo.start_proxy_listener {
    listen = '127.0.0.1:0',
    require_auth = true,
  }
end)

kumo.on('proxy_server_auth_1929', function(username, password, conn_meta)
  -- conn_meta is a table with peer_address and local_address
  -- Simple auth: accept testuser/testpass
  return username == 'testuser' and password == 'testpass'
end)
