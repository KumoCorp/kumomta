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
local listener_domains = require 'policy-extras.listener_domains'

local docker_utils = require 'docker_utils'

local shaping = require 'policy-extras.shaping'

local DOCKER_NETWORK = docker_utils.resolve_docker_network()

-- We need to ensure that we publish logs to each individual
-- tsa service node, and read data from all of them.
-- We resolve the tsa DNS record to obtain the list of IPs;
-- this requires that the service be deployed with
-- `endpoint_mode: dnsrr`.
-- NOTE: if you subsequently scale the number of tsa replicas,
-- a given kumod instance will have mismatching information
-- about them until it is restarted and shaper.setup_publish()
-- is called again.
local tsa_publish = docker_utils.resolve_tsa_endpoints()

local shaper = shaping:setup_with_automation {
  publish = tsa_publish,
  subscribe = tsa_publish,
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
}

--[[ Start of INIT section ]]
--

kumo.on('init', function()
  kumo.configure_redis_throttles {
    node = 'redis://redis/',
  }

  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2525',
    relay_hosts = { '127.0.0.0/24', '::1', DOCKER_NETWORK },
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    trusted_hosts = { '127.0.0.0/24', '::1', DOCKER_NETWORK },
  }

  -- While this example doesn't actively provide persistent
  -- storage management, this is a nod towards enabling it
  -- by putting both the spool and the logs in a slot-specific
  -- location.
  -- That allows another instance of a task to pick up the
  -- spool/logs if the orchestrator re-provisions the slot
  -- to another machine, provided that the administrator
  -- has configured some kind of persistent mount point
  -- for /var/spool/kumomta and/or /var/log/kumomta
  local slot = os.getenv 'SWARM_SLOT'

  kumo.define_spool {
    name = 'data',
    path = string.format('/var/spool/kumomta/data-%d', slot),
  }

  kumo.define_spool {
    name = 'meta',
    path = string.format('/var/spool/kumomta/meta-%d', slot),
  }

  kumo.configure_local_logs {
    log_dir = string.format('/var/log/kumomta/logs-%d', slot),
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

kumo.on(
  'get_listener_domain',
  listener_domains:setup { '/opt/kumomta/etc/policy/listener_domains.toml' }
)

--[[ Start of Non-INIT level config ]]
--
-- PLEASE read https://docs.kumomta.com/ for extensive documentation on customizing this config.
--[[ End of Non-INIT level config ]]
