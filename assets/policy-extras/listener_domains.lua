local mod = {}
local kumo = require 'kumo'
local utils = require 'policy_extras.policy_utils'

local function load_data_from_file(file_name, target)
  local data = utils.load_json_or_toml_file(file_name)
  for key, value in pairs(data) do
    -- Check that the value is a valid domain spec
    kumo.make_listener_domain(value)
    -- Store the original value
    target[key] = value
  end
end

local function load_data(data_files)
  local dmap = kumo.domain_map.new()
  for _, file_name in ipairs(data_files) do
    load_data_from_file(file_name, dmap)
  end
  return dmap
end

--[[
Usage:

Create a `/opt/kumomta/etc/listener_domains.toml` file with
contents like:

```toml
["example.com"]
# allow relaying mail from anyone, so long as it is
# addressed to example.com
relay_to = true

["bounce.example.com"]
# accept and log OOB bounce reports sent to bounce.example.com
log_oob = true

["fbl.example.com"]
# accept and log ARF feedback reports sent to fbl.example.com
log_arf = true

["send.example.com"]
# relay to anywhere, so long as the sender domain is send.example.com
# and the connected peer matches one of the listed CIDR blocks
relay_from = { '10.0.0.0/24' }

# wildcards are permitted. This will match
# <anything>.example.com that doesn't have
# another non-wildcard entry explicitly
# listed in this set of domains.
# Note that "example.com" won't match
# "*.example.com".
[#*.example.com#]
# You can specify multiple options if you wish
log_oob = true
log_arf = true
relay_to = true

# and you can explicitly set options to false to
# essentially exclude an entry from a wildcard
["www.example.com"]
relay_to = false
log_arf = false
log_oob = false
```

Then in your policy:

```
local listener_domains = require 'policy_extras.listener_domains'

kumo.on('get_listener_domain', listener_domains:setup({'/opt/kumomta/etc/listener_domains.toml'}))
```


You can use multiple data files, and they can be either toml
or json so long as they have the structure described above;
the keys are domain names with optional wildcard domains
being supported.  The contents of a section are any valid
key/value pair supported by kumo.make_listener_domain; the
section value will be passed to kumo.make_listener_domain.

]]
function mod:setup(data_files)
  local cached_load_data = kumo.memoize(load_data, {
    name = 'listener_domains_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  local function get_listener_domain(domain_name)
    local dmap = cached_load_data(data_files)
    return kumo.make_listener_domain(dmap[domain_name])
  end

  return get_listener_domain
end

return mod
