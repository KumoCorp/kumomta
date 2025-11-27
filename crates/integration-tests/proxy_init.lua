local kumo = require 'kumo'

kumo.on('proxy_init', function()
  kumo.start_proxy_listener {
    listen = '127.0.0.1:0',
  }
end)
