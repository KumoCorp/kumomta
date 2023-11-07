local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
Usage:

Create a `/opt/kumomta/etc/shaping.json` with contents like:

```json
{
  "gmail.com": {
    // "mx_rollup": false, // if false, is NOT shared with other domains with same mx
    "connection_limit": 3,

    // Entries that are themselves objects are considered to be
    // source configuration. These will take precendence over
    // the more general configuration for the domain/site
    "sources": {
      "source-0": {
        "connection_limit": 5
      }
    }
  }
}
```

---
local shaping = require 'policy-extras.shaping'
kumo.on('get_egress_path_config', shaping:setup('/opt/kumomta/etc/shaping.json'))
---

Alternatively, you could use a TOML file instead.
]]

function mod:setup(extra_files)
  return self:setup_with_automation({
    extra_files = extra_files,
    publish = {},
    subscribe = {},
  }).get_egress_path_config
end

local function load_shaping_data(file_names)
  -- print('Loading shaping data from ', kumo.json_encode(file_names))
  local result = kumo.shaping.load(file_names)
  local warnings = result:get_warnings()
  for _, warn in ipairs(warnings) do
    print(warn)
  end
  -- print('Computed', kumo.json_encode_pretty(result))
  return result
end

local function should_enq(publish, msg, hook_name)
  local params = publish[hook_name]
  if not params then
    -- User defined log hook that is not part of shaping.lua
    return
  end

  local log_record = msg:get_meta 'log_record'
  -- avoid overlap with other logs
  if log_record.reception_protocol == 'LogRecord' then
    return false
  end

  -- We only want to log if the event isn't one of our
  -- publishing events
  for name, _ in pairs(publish) do
    if name == log_record.queue then
      -- It's one of our log hooks; don't queue this one
      return false
    end
  end

  -- It was not destined to any of our hooks, so we can safely
  -- queue this one without triggering a cycle
  msg:set_meta('queue', hook_name)
  return true
end

local function construct_publisher(publish, domain)
  local connection = {}
  local client = kumo.http.build_client {}
  function connection:send(message)
    local response = client
      :post(string.format('%s/publish_log_v1', publish.endpoint))
      :header('Content-Type', 'application/json')
      :body(message:get_data())
      :send()

    local disposition = string.format(
      '%d %s: %s',
      response:status_code(),
      response:status_reason(),
      response:text()
    )

    if response:status_is_success() then
      return disposition
    end

    -- retry later
    kumo.reject(400, disposition)
  end
  return connection
end

local function get_queue_cfg(publish, domain, tenant, campaign)
  for _, data in pairs(publish) do
    if data.hook_name == domain then
      return kumo.make_queue_config {
        protocol = {
          custom_lua = {
            constructor = data.constructor,
          },
        },
      }
    end
  end
end

--[[
local shaper = shaping:setup_with_automation {
  publish = {"http://10.0.0.1:8008"},
  subscribe = {"http://10.0.0.1:8008"},
  -- this needs to list any files that hold your custom shaping rules; should match
  -- the additional files beyond /opt/kumomta/share/policy-extras/shaping.toml in your
  -- tsa config
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
}

kumo.on('init', function()
  shaper.setup_publish()
end)

kumo.on('get_egress_path_config', shaper.get_egress_path_config)
]]
function mod:setup_with_automation(options)
  local cached_load_data = kumo.memoize(load_shaping_data, {
    name = 'shaping_data',
    ttl = '1 minute',
    capacity = 10,
  })

  local file_names = {
    -- './assets/policy-extras/shaping.toml',
    '/opt/kumomta/share/policy-extras/shaping.toml',
  }
  if options.extra_files then
    for _, filename in ipairs(options.extra_files) do
      table.insert(file_names, filename)
    end
  end
  if options.subscribe then
    for _, url in ipairs(options.subscribe) do
      table.insert(
        file_names,
        string.format('%s/get_config_v1/shaping.toml', url)
      )
    end
  end

  local publish = {}
  for _, destination in ipairs(options.publish) do
    -- Generate the hook name and constructor name and
    -- keep that info in a more structured form
    local hook_name = string.format('%s.tsa.kumomta', destination)
    local constructor = string.format('make.%s', hook_name)

    publish[hook_name] = {
      endpoint = destination,
      hook_name = hook_name,
      constructor = constructor,
    }

    -- Since we own naming the constructor events, we can make
    -- them unique without fear of colliding with user-provided
    -- events, so we can simply bind the event handlers here
    -- without returning them to the caller to deal with
    kumo.on(constructor, function(domain, _tenant, _campaign)
      return construct_publisher(publish[hook_name], domain)
    end)
  end

  local function setup_publish()
    for _, params in pairs(publish) do
      kumo.configure_log_hook {
        name = params.hook_name,
        per_record = {
          -- Don't feed reception data to the daemon; we're
          -- only interested in data that flows back to us
          -- from after the point of reception
          Reception = {
            enable = false,
          },
        },
      }
    end
  end

  local function get_egress_path_config(domain, egress_source, site_name)
    local data = cached_load_data(file_names, options.subscribe)
    local params =
      data:get_egress_path_config(domain, egress_source, site_name)

    --[[
    print(
      'going to make egress path',
      domain,
      egress_source,
      site_name,
      kumo.json_encode(params)
    )
    ]]

    return kumo.make_egress_path(params)
  end

  -- Setup the webhook publisher to the TSA daemon.
  -- Since each destination has a unique domain name,
  -- the implementation of get_queue_cfg can simply
  -- match that name and return the full configuration
  -- for it; there is no need for user config to need
  -- to mutate it so we can register a handler here without
  -- exposing the handler to the user's config, make things
  -- just a little simpler for them.
  kumo.on(
    'get_queue_config',
    function(domain, tenant, campaign, routing_domain)
      return get_queue_cfg(publish, domain, tenant, campaign)
    end
  )

  kumo.on('should_enqueue_log_record', function(msg, hook_name)
    return should_enq(publish, msg, hook_name)
  end)

  return {
    get_egress_path_config = get_egress_path_config,
    should_enqueue_log_record = function(msg, hook_name)
      -- deprecated: no longer needed as we register a should_enqueue_log_record
      -- handler above.
      -- This is preserved for backwards compatibility; when
      -- called, it does nothing.
      -- TODO: remove me after next release.
      return
    end,
    setup_publish = setup_publish,
    get_queue_config = function(domain, tenant, campaign, routing_domain)
      -- deprecated: no longer needed as we register a get_queue_config
      -- handler above.
      -- This is preserved for backwards compatibility; when
      -- called, it does nothing.
      -- TODO: remove me after next release.
    end,
  }
end

return mod
