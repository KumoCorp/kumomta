--[[
########################################################
  KumoMTA minimal Send Policy
  (Save this as /opt/kumomta/etc/policy/init.lua for systemd automation)
  This config policy defines KumoMTA with a minimal
  set of modifications from default.
  Please read the docs at https://docs.kumomta.com/
  For detailed configuration instructions.
########################################################
]]
--
local kumo = require 'kumo'
--[[ Start of INIT section ]]
--

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:8000',
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumomta/data',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumomta/meta',
  }

  kumo.configure_local_logs {
    log_dir = '/var/log/kumomta',
  }
end)
--[[ End of INIT Section ]]

--[[ Start of Non-INIT level config ]]
--
-- PLEASE read https://docs.kumomta.com/ for extensive documentation on customizing this config.
--[[ End of Non-INIT level config ]]
