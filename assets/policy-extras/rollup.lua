local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function all_hosts_match_suffix(mx, suffix)
  for _, host in ipairs(mx.hosts) do
    if not utils.ends_with(host, suffix) then
      return false
    end
  end
  return true
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
      if all_hosts_match_suffix(mx, suffix) then
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
