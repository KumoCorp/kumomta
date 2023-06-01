local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function load_data_from_file(file_name, target)
  local data = utils.load_json_or_toml_file(file_name)

  for source, params in pairs(data.source) do
    target.sources[source] = target.sources[source] or {}
    utils.merge_into(params, target.sources[source])
  end

  for pool, pool_def in pairs(data.pool) do
    for pool_source, params in pairs(pool_def) do
      target.pools[pool] = target.pools[pool]
        or {
          entries = {},
        }

      params.name = pool_source
      table.insert(target.pools[pool].entries, params)
    end
  end
end

local function load_data(data_files)
  local target = {
    sources = {},
    pools = {},
  }

  for _, file_name in ipairs(data_files) do
    load_data_from_file(file_name, target)
  end

  return target
end

--[[
Usage:

Create a `/opt/kumomta/etc/sources.toml` file with
contents like:

```toml
[source."ip-1"]
source_address = "10.0.0.1"

[source."ip-2"]
source_address = "10.0.0.2"

[source."ip-3"]
source_address = "10.0.0.3"

# Pool containing just ip-1, which has weight=1
[pool."BestReputation"]
[pool."BestReputation"."ip-1"]

# Pool with multiple ips
[pool."MediumReputation"]

[pool."MediumReputation"."ip-2"]
weight = 2

# We're warming up ip-3, so use it less frequently than ip-2
[pool."MediumReputation"."ip-3"]
weight = 1
```

Then in your policy:

```
local sources = require 'policy-extras.sources'

sources:setup({'/opt/kumomta/etc/sources.toml'})
```
]]

function mod:setup(data_files)
  local cached_load_data = kumo.memoize(load_data, {
    name = 'sources_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  kumo.on('get_egress_source', function(source_name)
    local data = cached_load_data(data_files)
    return kumo.make_egress_source(data.sources[source_name])
  end)

  kumo.on('get_egress_pool', function(pool_name)
    local data = cached_load_data(data_files)
    return kumo.make_egress_poll(data.pools[pool_name])
  end)
end

return mod
