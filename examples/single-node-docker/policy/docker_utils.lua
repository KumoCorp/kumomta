local mod = {}
local kumo = require 'kumo'

-- Resolve our hostname, then derive the docker network from that.
-- This assumes that the network is a /24 and that HOSTNAME is
-- injected by docker and resolves to the container IP on that network
function mod.resolve_docker_network()
  local MY_IPS = kumo.dns.lookup_addr(os.getenv('HOSTNAME'))
  local DOCKER_NETWORK = string.match(MY_IPS[1], "^(.*)%.%d+$") .. ".0/24"
  return DOCKER_NETWORK
end

return mod
