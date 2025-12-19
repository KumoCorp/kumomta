-- Proxy configuration with optional authentication
-- Auth credentials are passed via environment variables
local kumo = require 'kumo'

local REQUIRE_AUTH = os.getenv 'KUMO_PROXY_REQUIRE_AUTH' == 'true'
local AUTH_USERNAME = os.getenv 'KUMO_PROXY_AUTH_USERNAME' or 'testuser'
local AUTH_PASSWORD = os.getenv 'KUMO_PROXY_AUTH_PASSWORD' or 'testpass'

kumo.on('proxy_init', function()
  kumo.start_proxy_listener {
    listen = '127.0.0.1:0',
    require_auth = REQUIRE_AUTH,
  }
end)

kumo.on('proxy_server_auth_rfc1929', function(username, password, conn_meta)
  -- Validate credentials against expected values
  return username == AUTH_USERNAME and password == AUTH_PASSWORD
end)
