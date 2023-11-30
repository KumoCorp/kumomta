local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
Usage:

Create a `/opt/kumomta/etc/queue.toml` with contents like:

```toml
# Allow optional scheduled sends based on this header
# https://docs.kumomta.com/reference/message/import_scheduling_header
scheduling_header = "X-Schedule"

# Set the tenant from this header
tenant_header = "X-Tenant"
remove_tenant_header = true

# Set the campaign from this header
campaign_header = "X-Campaign"
remove_campaign_header = true

# The tenant to use if no tenant_header is present
default_tenant = "default-tenant"

[tenant.'default-tenant']
egress_pool = 'pool-0'

[tenant.'mytenant']
# Which pool should be used for this tenant
egress_pool = 'pool-1'
# Override age based on tenant; this overrides
# settings at the domain level
max_age = '10 hours'
# Only the authorized identities are allowed to use this tenant
# via the tenant_header
require_authz = ["scott"]

# The default set of parameters
[queue.default]
max_age = '24 hours'

# Base settings for a given destination domain.
# These are overridden by more specific settings
# in a tenant or more specific queue
[queue.'gmail.com']
max_age = '22 hours'
retry_interval = '17 mins'

[queue.'gmail.com'.'mytenant']
# options here for domain=gmail.com AND tenant=mytenant for any unmatched campaign

[queue.'gmail.com'.'mytenant'.'welcome-campaign']
# options here for domain=gmail.com, tenant=mytenant, and campaign='welcome-campaign'
```

---
local queue_module = require 'policy-extras.queue'
local queue_helper = queue_module:setup({'/opt/kumomta/etc/queue.toml'})

kumo.on('smtp_server_message_received', function(msg)
  queue_helper:apply(msg)

  -- do your dkim signing here
end)

---
]]

local function parse_one(data)
  local options = {}
  local tenants = {}
  local queues = {}

  for k, v in pairs(data) do
    if k == 'tenant' then
      for tenant_name, tenant_options in pairs(v) do
        local tenant = tenants[tenant_name] or {}
        utils.merge_into(tenant_options, tenant)
        tenants[tenant_name] = tenant
      end
    elseif k == 'queue' then
      for domain_name, queue_options in pairs(v) do
        local domain_options = queues[domain_name] or {}
        utils.recursive_merge_into(queue_options, domain_options)
        queues[domain_name] = domain_options
      end
    else
      options[k] = v
    end
  end

  local result = {
    options = options,
    tenants = tenants,
    queues = queues,
  }

  return result
end

local function merge_data(loaded_files)
  local result = {}
  for _, data in ipairs(loaded_files) do
    utils.recursive_merge_into(parse_one(data), result)
  end
  -- print(kumo.json_encode_pretty(result))
  result.queues = kumo.domain_map.new(result.queues)
  return result
end

local function load_queue_config(file_names)
  local data = {}
  for _, file_name in ipairs(file_names) do
    table.insert(data, utils.load_json_or_toml_file(file_name))
  end

  return merge_data(data)
end

local function resolve_config(data, domain, tenant, campaign)
  -- print('resolve_config', domain, tenant, campaign)

  local params = {}

  local default_config = data.queues.default
  if default_config then
    utils.merge_into(default_config, params)
  end

  local domain_config = data.queues[domain]
  if domain_config then
    for k, v in pairs(domain_config) do
      if type(v) ~= 'table' then
        params[k] = v
      end
    end
  end

  local tenant_definition = data.tenants[tenant]
  if tenant_definition then
    for k, v in pairs(tenant_definition) do
      if type(v) ~= 'table' then
        params[k] = v
      end
    end
  end

  if domain_config then
    local tenant_config = domain_config[tenant]
    if tenant_config then
      for k, v in pairs(tenant_config) do
        if type(v) ~= 'table' then
          params[k] = v
        end
      end

      local campaign = tenant_config[campaign]

      if campaign then
        for k, v in pairs(campaign) do
          if type(v) ~= 'table' then
            params[k] = v
          end
        end
      end
    end
  end

  if utils.table_is_empty(params) then
    return nil
  end

  -- print(kumo.json_encode_pretty(params))
  return params
end

function mod:setup(file_names)
  return self:setup_with_options {
    skip_queue_config_hook = false,
    file_names = file_names,
  }
end

function mod:setup_with_options(options)
  local cached_load_data = kumo.memoize(load_queue_config, {
    name = 'queue_helper_data',
    ttl = '1 minute',
    capacity = 10,
  })

  local helper = {
    file_names = options.file_names,
  }

  function helper:resolve_config(domain, tenant, campaign)
    local data = cached_load_data(options.file_names)
    local params = resolve_config(data, domain, tenant, campaign)
    return params
  end

  if not options.skip_queue_config_hook then
    kumo.on(
      'get_queue_config',
      function(domain, tenant, campaign, _routing_domain)
        local data = cached_load_data(options.file_names)
        local params = resolve_config(data, domain, tenant, campaign)
        if params then
          return kumo.make_queue_config(params)
        end
      end
    )
  end

  function helper:apply(msg)
    local data = cached_load_data(options.file_names)
    if data.options.scheduling_header then
      msg:import_scheduling_header(data.options.scheduling_header, true)
    end
    if data.options.campaign_header then
      local campaign =
        msg:get_first_named_header_value(data.options.campaign_header)
      if campaign then
        msg:set_meta('campaign', campaign)
        if data.options.remove_campaign_header then
          msg:remove_all_named_headers(data.options.campaign_header)
        end
      end
    end
    local tenant = nil
    local tenant_source = nil
    if data.options.tenant_header then
      tenant = msg:get_first_named_header_value(data.options.tenant_header)
      if tenant then
        tenant_source =
          string.format("'%s' header", data.options.tenant_header)
        if data.options.remove_tenant_header then
          msg:remove_all_named_headers(data.options.tenant_header)
        end
      end
    end
    if not tenant and data.options.default_tenant then
      tenant = data.options.default_tenant
      tenant_source = 'default_tenant option'
    end
    if tenant then
      local tenant_config = data.tenants[tenant]

      if not tenant_config then
        kumo.reject(
          500,
          string.format(
            "tenant '%s' specified by %s is not defined in your queue config",
            tenant,
            tenant_source
          )
        )
      end

      if tenant_config.require_authz then
        local my_auth = msg:get_meta 'authz_id'

        if not my_auth then
          kumo.reject(
            500,
            string.format("tenant '%s' requires SMTP AUTH", tenant)
          )
        end

        if not utils.table_contains(tenant_config.require_authz, my_auth) then
          kumo.reject(
            500,
            string.format(
              "authz_id '%s' is not authorized to send as tenant '%s'",
              my_auth,
              tenant
            )
          )
        end
      end

      msg:set_meta('tenant', tenant)
    end
  end

  return helper
end

--[[
Run some basic unit tests for the data parsing/merging; use it like this:

```
local queue_module = require 'policy-extras.queue'
queue_module:test()
```
]]
function mod:test()
  local base_data = [=[
default_tenant = 'mytenant'

[queue.default]
max_age = '24 hours'

[tenant.'mytenant']
egress_pool = 'tpool'

[queue.'gmail.com'.'mytenant']
egress_pool = "foo"
]=]

  local user_data = [=[
[queue.'gmail.com']
max_age = '6 hours'

[queue.'gmail.com'.'mytenant'.'campaign']
egress_pool = "bar"
]=]

  local data = merge_data {
    kumo.toml_parse(base_data),
    kumo.toml_parse(user_data),
  }

  utils.assert_eq(
    resolve_config(data, 'foo.com', nil, nil, nil).max_age,
    '24 hours'
  )
  utils.assert_eq(
    resolve_config(data, 'gmail.com', nil, nil, nil).max_age,
    '6 hours'
  )
  utils.assert_eq(
    resolve_config(data, 'gmail.com', nil, nil, nil).egress_pool,
    nil
  )
  utils.assert_eq(
    resolve_config(data, 'example.com', 'mytenant', nil, nil).egress_pool,
    'tpool'
  )
  utils.assert_eq(
    resolve_config(data, 'gmail.com', 'mytenant', nil, nil).egress_pool,
    'foo'
  )
  utils.assert_eq(
    resolve_config(data, 'gmail.com', 'mytenant', 'campaign', nil).egress_pool,
    'bar'
  )
end

return mod
