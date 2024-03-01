local mod = {}
local kumo = require 'kumo'

-- Logically equivalent to kumo.dns.lookup_addr, except that
-- function doesn't seem to work reliably with the docker service
-- discovery DNS during early container startup.
-- Instead this function uses the getent utility to resolve
-- via the system resolver.
-- If the endpoint_mode for the service is dnsrr this will
-- return a list of the IPs of the constituent services.
-- Otherwise it will return the load balancer IP
local function resolve_docker_name(name)
  local result = {}
  local f = io.popen('getent ahostsv4 ' .. name, 'r')
  for line in f:lines() do
    local ip = string.match(line, '^(%d+%.%d+%.%d+%.%d+)%s+STREAM')
    if ip then
      table.insert(result, ip)
    end
  end
  -- print('resolve_docker_name', name, kumo.json_encode(result))
  return result
end

-- Employ a cache around the system resolver for resolve_docker_name
mod.resolve_docker_name = kumo.memoize(resolve_docker_name, {
  name = 'resolve_docker_name',
  -- Not too long, so that we have lower latency for picking up
  -- scaling changes
  ttl = '1 minutes',
  capacity = 1024,
})

-- Resolve our hostname, then derive the docker network from that.
-- This assumes that the network is a /24 and that HOSTNAME is
-- injected by docker and resolves to the container IP on that network.
local function resolve_docker_network()
  local hostname = os.getenv 'HOSTNAME'
  if not hostname then
    error 'HOSTNAME is not set in the environment'
  end
  local MY_IPS = mod.resolve_docker_name(hostname)
  local DOCKER_NETWORK = string.match(MY_IPS[1], '^(.*)%.%d+$') .. '.0/24'
  return DOCKER_NETWORK
end

mod.resolve_docker_network = kumo.memoize(resolve_docker_network, {
  name = 'resolve_docker_network',
  ttl = '1 hour',
  capacity = 1,
})

-- We resolve the tsa DNS record to obtain the list of IPs;
-- this requires that the service be deployed with
-- `endpoint_mode: dnsrr`.
-- Map the list of IPs into a list of TSA HTTP endpoints.
function mod.resolve_tsa_endpoints()
  local ips = mod.resolve_docker_name 'tsa'
  local result = {}
  for ip, _ in pairs(ips) do
    table.insert(result, string.format('http://%s:8008', ip))
  end
  return result
end

return mod
