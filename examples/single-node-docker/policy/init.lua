--[[
########################################################
  KumoMTA Docker Basic Policy
  This config policy defines KumoMTA with a minimal
  set of modifications from default.
  Please read the docs at https://docs.kumomta.com/
  For detailed configuration instructions.
########################################################
]]
--
local kumo = require 'kumo'
local docker_utils = require 'docker_utils'

local shaping = require 'policy-extras.shaping'

local DOCKER_NETWORK = docker_utils.resolve_docker_network()

-- The compose file causes tsa-daemon to be started and assigned
-- the DNS name `tsa`. We use that name to resolve the daemon here.
local shaper = shaping:setup_with_automation {
  publish = { 'http://tsa:8008' },
  subscribe = { 'http://tsa:8008' },
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
}


--[[ Start of INIT section ]]
--

kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2525',
    relay_hosts = { '127.0.0.0/24', '::1', DOCKER_NETWORK },
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    trusted_hosts = { '127.0.0.0/24', '::1', DOCKER_NETWORK },
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
    -- Flush logs every 10 seconds.
    -- You may wish to set a larger value in your production
    -- configuration; this lower value makes it quicker to see
    -- logs while you are first getting set up.
    max_segment_duration = '10s',
  }

  -- Configure publishing of logs to automation daemon
  shaper.setup_publish()
end)
--[[ End of INIT Section ]]

--[[ Start of Non-INIT level config ]]
--
-- PLEASE read https://docs.kumomta.com/ for extensive documentation on customizing this config.
--[[ End of Non-INIT level config ]]
