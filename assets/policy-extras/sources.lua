local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function load_data_from_file(file_name, target)
  local data = utils.load_json_or_toml_file(file_name)

  for source, params in pairs(data.source or {}) do
    target.sources[source] = target.sources[source]
      or {
        name = source,
      }
    utils.merge_into(params, target.sources[source])
  end

  for pool, pool_def in pairs(data.pool or {}) do
    for pool_source, params in pairs(pool_def) do
      target.pools[pool] = target.pools[pool]
        or {
          name = pool,
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

  -- print(kumo.json_encode_pretty(target))

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
  if mod.CONFIGURED then
    error 'sources module has already been configured'
  end

  local cached_load_data = kumo.memoize(load_data, {
    name = 'sources_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  mod.CONFIGURED = {
    data_files = data_files,
    get_data = function()
      return cached_load_data(data_files)
    end,
  }

  kumo.on('get_egress_source', function(source_name)
    local data = cached_load_data(data_files)
    local params = data.sources[source_name]
    --[[
    print(
      string.format('source %s: %s', source_name, kumo.json_encode(params))
    )
    ]]
    return kumo.make_egress_source(params)
  end)

  kumo.on('get_egress_pool', function(pool_name)
    local data = cached_load_data(data_files)
    local params = data.pools[pool_name]
    -- print(string.format('pool %s: %s', pool_name, kumo.json_encode(params)))
    return kumo.make_egress_pool(params)
  end)
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local data = mod.CONFIGURED.get_data()
  local failed = false

  function show_context()
    if failed then
      return
    end
    failed = true
    print 'Issues found in the combined set of sources files:'
    for _, file_name in ipairs(mod.CONFIGURED.data_files) do
      if type(file_name) == 'table' then
        print ' - (inline table)'
      else
        print(string.format(' - %s', file_name))
      end
    end
  end

  for source, params in pairs(data.sources) do
    local status, err = pcall(kumo.make_egress_source, params)
    if not status then
      show_context()
      print(err)
      kumo.validation_failed()
    end
  end
  for pool, params in pairs(data.pools) do
    local status, err = pcall(kumo.make_egress_pool, params)
    if not status then
      show_context()
      print(err)
      kumo.validation_failed()
    end

    for _, entry in ipairs(params.entries) do
      if not data.sources[entry.name] then
        show_context()
        print(
          string.format(
            "pool '%s' references source '%s' which is not defined",
            pool,
            entry.name
          )
        )
      end
    end
  end
end)

return mod
