local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function all_hosts_match_suffix(hosts, suffix)
  for _, host in ipairs(hosts) do
    if not utils.ends_with(host, suffix) then
      return false
    end
  end
  return true
end

local function compute_ip_rollup_mx_list(domain, routing_domain)
  if routing_domain == nil then
    return nil
  end
  if not utils.ends_with(routing_domain, '.ip_rollup') then
    return nil
  end

  local mx = kumo.dns.lookup_mx(domain)

  local lowest_pref = nil
  local hosts = nil

  for pref, pref_hosts in pairs(mx.by_pref) do
    pref = tonumber(pref)
    if lowest_pref == nil or pref < lowest_pref then
      lowest_pref = pref
      hosts = pref_hosts
    end
  end

  local addrs = {}
  -- The host names are pre-sorted by lookup_mx
  for _, host in ipairs(hosts) do
    local host_addrs = kumo.dns.lookup_addr(host)
    -- The addresses are in an unspecified order.
    -- Sort them so that we produce a consistent list
    -- and hash consistently with the resulting ready queue name
    table.sort(host_addrs)
    for _, a in ipairs(host_addrs) do
      table.insert(addrs, string.format('[%s]', a))
    end
  end

  return addrs
end

function mod.apply_ip_rollup_to_queue_config(domain, routing_domain, params)
  local mx_list = compute_ip_rollup_mx_list(domain, routing_domain)
  if mx_list then
    params.protocol = {
      smtp = {
        mx_list = mx_list,
      },
    }
  end
end

local function compute_ip_rollup(domain, mapping)
  local mx = kumo.dns.lookup_mx(domain)

  local lowest_pref = nil
  local hosts = nil

  -- Extract the lowest pref value (highest priority) set
  -- of mx hosts from the mx records
  for pref, pref_hosts in pairs(mx.by_pref) do
    pref = tonumber(pref)
    if lowest_pref == nil or pref < lowest_pref then
      lowest_pref = pref
      hosts = pref_hosts
    end
  end

  -- Find a matching mapping entry
  local routing_domain = nil
  for suffix, routing_domain in pairs(mapping) do
    if all_hosts_match_suffix(hosts, suffix) then
      return routing_domain
    end
  end
  return nil
end

--[[
This function performs ip-based rollup for domains whose
MX records match certain suffixies.

The mapping parameter must be a map of mx hostname suffix
to a synthetic routing domain name, which must also end
with `.ip_rollup`.

The recipient domain is resolved for the supplied message,
and its lowest preferenced (eg: highest priority) mx records
are compared against the hostname suffix.  If they all match,
then the message has its routing domain set to the routing
domain.

In order to use this successfully, you must also arrange
for your get_queue_config event to update the queue parameters:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local params = {}
  rollup.apply_ip_rollup_to_queue_config(domain, routing_domain, params
  return kumo.make_queue_config(params)
end)

kumo.on('smtp_server_message_received', function(msg)
  rollup.reroute_using_ip_rollup(msg, {
    ['.mail.protection.outlook.com.'] = 'outlook.ip_rollup',
  })
end)
```

To apply shaping to the result, if you are using the shaping helper,
you will need to use the ip rollup routing domain:

```toml
["outlook.ip_rollup"]
mx_rollup = false
# shaping parameters here
```

]]
function mod.reroute_using_ip_rollup(msg, mapping)
  local status, err = pcall(function()
    local recip_domain = msg:recipient().domain
    local routing_domain = compute_ip_rollup(recip_domain, mapping)
    msg:set_meta('routing_domain', routing_domain)
  end)
  if not status then
    -- Log, but otherwise ignore failure to resolve the domain
    print(string.format('while checking ip_rollup: %s', err))
  end
end

--[[
This function considers a mapping of MX hostname suffixes
to equivalent routing domains.

The MX for the recipient domain is resolved, and the set
of MX host names are checked against the suffixes.
If every hostname matches a suffix in the mapping, it is
consider to be an overall match, and the `routing_domain`
meta value is set to the corresponding domain.

This causes the mail for the recipients to have separate
scheduled queues but to use the same ready_queue and
site_name when it comes to delivering the mail, allowing
you to enforce traffic shaping rules on the same entity.

To use, place something like this in your
`smtp_server_message_received` and/or
`http_message_received` event handlers:

rollup.reroute_based_on_mx_host_suffix(msg, {
  [".olc.protection.outlook.com."] = "outlook.com",
})

]]
function mod.reroute_based_on_mx_host_suffix(msg, mapping)
  local status, err = pcall(function()
    local recip_domain = msg:recipient().domain
    local mx = kumo.dns.lookup_mx(recip_domain)
    for suffix, routing_domain in pairs(mapping) do
      if all_hosts_match_suffix(mx.hosts, suffix) then
        if recip_domain ~= routing_domain then
          msg:set_meta('routing_domain', routing_domain)
        end
        return
      end
    end
  end)
  if not status then
    -- Log, but otherwise ignore failure to resolve the domain
    print(string.format('while checking mx host pattern: %s', err))
  end
end

return mod
