-- This config acts as a sink that will discard all received mail
local kumo = require 'kumo'
package.path = 'assets/?.lua;' .. package.path
local utils = require 'policy-extras.policy_utils'

local SINK_DATA_FILE = os.getenv('SINK_DATA') or '/opt/kumomta/etc/policy/responses.toml'

-- Resolve our hostname, then derive the docker network from that.
-- This assumes that the network is a /24 and that HOSTNAME is
-- injected by docker and resolves to the container IP on that network
local function resolve_docker_network()
  local MY_IPS = kumo.dns.lookup_addr(os.getenv('HOSTNAME') or 'localhost')
  local DOCKER_NETWORK = string.match(MY_IPS[1], '^(.*)%.%d+$') .. '.0/24'
  return DOCKER_NETWORK
end

kumo.on('init', function()
  kumo.configure_accounting_db_path(os.tmpname())
  kumo.set_config_monitor_globs({SINK_DATA_FILE})
  local DOCKER_NETWORK = resolve_docker_network()
  local SINK_PORT = os.getenv('SINK_PORT') or '25'
  kumo.start_esmtp_listener {
    listen = '0:' .. SINK_PORT,
    -- Explicitly an open relay, because we want to sink everything
    relay_hosts = { '0.0.0.0/0' },
    banner = 'This system will sink and discard all mail',
  }

  local SINK_HTTP = os.getenv('SINK_HTTP') or '8000'
  kumo.start_http_listener {
    listen = '0.0.0.0:' .. SINK_HTTP,
    trusted_hosts = { '127.0.0.1', '::1', DOCKER_NETWORK },
  }

  -- Define spool locations
  -- This is unused by this config, but we are required to
  -- define a default spool location.

  local spool_dir = os.getenv('SINK_SPOOL') or '/var/spool/kumomta'

  for _, name in ipairs { 'data', 'meta' } do
    kumo.define_spool {
      name = name,
      path = spool_dir .. '/' .. name,
    }
  end

  -- No logs are configured: we don't need them
end)

-- Load and parse the responses.toml data and resolve the configuration
-- for a given domain
local function load_data_for_domain(domain)
  local data = kumo.toml_load(SINK_DATA_FILE)
  local config = data.domain[domain] or data.default
  config.bounces = data.bounce[domain] or { { code = 550, msg = 'boing!' } }
  config.defers = data.defer[domain] or { { code = 451, msg = 'later!' } }
  return config
end

-- Cache the result of a load_data_for_domain call
local resolve_domain = kumo.memoize(load_data_for_domain, {
  name = 'response-data-cache',
  ttl = '1 hour',
  capacity = 100,
})

kumo.on('smtp_server_message_received', function(msg)
  local recipient = msg:recipient()

  -- Do any special responses requested by the client
  if string.find(recipient.user, 'tempfail') then
    kumo.reject(400, 'tempfail requested')
  end
  if string.find(recipient.user, 'permfail') then
    kumo.reject(500, 'permfail requested')
  end
  if utils.starts_with(recipient.user, '450-') then
    kumo.reject(450, 'you said ' .. recipient.user)
  end
  if utils.starts_with(recipient.user, '250-') then
    msg:set_meta('queue', 'null')
    return
  end

  -- Now any general bounce responses based on the toml file
  local domain = recipient.domain
  local config = resolve_domain(domain)

  local d100 = math.random(100)
  local selection = nil
  if d100 < config.bounce then
    selection = config.bounces
  elseif d100 < config.bounce + config.defer then
    selection = config.defers
  end

  if selection then
    local choice = selection[math.random(#selection)]
    kumo.reject(choice.code, choice.msg)
  end

  -- Finally, accept and discard any messages that haven't
  -- been rejected already
  msg:set_meta('queue', 'null')
end)

kumo.on('http_message_generated', function(msg)
  -- Accept and discard all messages
  msg:set_meta('queue', 'null')
end)
