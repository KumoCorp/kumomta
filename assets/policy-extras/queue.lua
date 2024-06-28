local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
local typing = require 'policy-extras.typing'

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

-- Return true if the `name` and `value` pair provided
-- is a valid queue configuration option.
-- It does this by asking the internal rust code to validate it.
local function is_queue_config_option(name, value)
  local p = { [name] = value }
  local status, err = pcall(kumo.make_queue_config, p)
  if not status then
    if tostring(err):find 'invalid type' then
      error(err, 3)
    end
  end
  return status
end

local Bool, List, Map, Option, Record, String =
  typing.boolean,
  typing.list,
  typing.map,
  typing.option,
  typing.record,
  typing.string

local CampaignConfig = Record('CampaignConfig', {
  _dynamic = is_queue_config_option,
  overall_max_message_rate = Option(String),
  routing_domain = Option(String),
})

local TenantWithCampaignConfig = Record('TenantWithCampaignConfig', {
  _dynamic = is_queue_config_option,
  overall_max_message_rate = Option(String),
  routing_domain = Option(String),

  campaigns = Option(Map(String, CampaignConfig)),
})

local DomainConfig = Record('DomainConfig', {
  _dynamic = is_queue_config_option,
  overall_max_message_rate = Option(String),
  routing_domain = Option(String),

  tenants = Option(Map(String, TenantWithCampaignConfig)),
})

local TenantConfig = Record('TenantConfig', {
  campaigns = Option(Map(String, CampaignConfig)),
  _dynamic = is_queue_config_option,
  require_authz = Option(List(String)),
  overall_max_message_rate = Option(String),
})

local QueueHelperConfig = Record('QueueHelperConfig', {
  scheduling_header = Option(String),
  tenant_header = Option(String),
  remove_tenant_header = Option(Bool),
  campaign_header = Option(String),
  remove_campaign_header = Option(Bool),
  default_tenant = Option(String),

  tenants = Option(Map(String, TenantConfig)),
  queues = Option(Map(String, DomainConfig)),
})

local function parse_tenant_with_campaign(data)
  local tenant = TenantWithCampaignConfig {
    campaigns = {},
  }
  -- print('parse_tenant_with_campaign', kumo.serde.json_encode_pretty(data))
  for k, v in pairs(data) do
    -- Since we allow an arbitrary key to map to a campaign, and a mixture
    -- of known-to-lua and known-to-rust options, short of replicating
    -- the logic in the typing module here, the best option is just to
    -- try to assign it, and if it raises an error, then it must be
    -- a campaign value.
    local status, result = pcall(function()
      tenant[k] = v
    end)
    if not status then
      tenant.campaigns[k] = v
    end
  end

  return tenant
end

local function parse_domain(data)
  local domain = DomainConfig {
    tenants = {},
  }
  -- print('parse_domain', kumo.serde.json_encode_pretty(data))
  for k, v in pairs(data) do
    -- Since we allow an arbitrary key to map to a tenant, and a mixture
    -- of known-to-lua and known-to-rust options, short of replicating
    -- the logic in the typing module here, the best option is just to
    -- try to assign it, and if it raises an error, then it must be
    -- a tenant value.
    local status, result = pcall(function()
      domain[k] = v
    end)
    if not status then
      domain.tenants[k] = parse_tenant_with_campaign(v)
    end
  end

  return domain
end

local function parse_config(data)
  local result = QueueHelperConfig {
    tenants = {},
    queues = {},
  }

  for k, v in pairs(data) do
    if k == 'tenant' then
      for tenant_name, tenant_options in pairs(v) do
        local tenant = result.tenants[tenant_name] or {}
        utils.merge_into(tenant_options, tenant)
        result.tenants[tenant_name] = tenant
      end
    elseif k == 'queue' then
      for domain_name, queue_options in pairs(v) do
        local domain_options = result.queues[domain_name] or {}
        utils.recursive_merge_into(
          parse_domain(queue_options),
          domain_options
        )
        result.queues[domain_name] = domain_options
      end
    else
      result[k] = v
    end
  end

  return result
end

local function merge_data(loaded_files, no_compile)
  local result = QueueHelperConfig {}
  for _, data in ipairs(loaded_files) do
    utils.recursive_merge_into(parse_config(data), result)
  end
  -- print(kumo.json_encode_pretty(result))
  if not no_compile then
    result.queues = kumo.domain_map.new(result.queues)
  end
  return result
end

local function load_queue_config(file_names, no_compile)
  local data = {}
  for _, file_name in ipairs(file_names) do
    table.insert(data, utils.load_json_or_toml_file(file_name))
  end

  return merge_data(data, no_compile)
end

-- Resolve the merged value of the config that matches the provided
-- domain, tenant and campaign.
-- If allow_all is true, all fields are merged and returned.
-- Otherwise (the default), only valid make_queue_config option values
-- are returned.
local function resolve_config(data, domain, tenant, campaign, allow_all)
  -- print('resolve_config', domain, tenant, campaign)

  local params = {}

  local default_config = data.queues.default
  if default_config then
    for k, v in pairs(default_config) do
      if allow_all or is_queue_config_option(k, v) then
        params[k] = v
      end
    end
  end

  local domain_config = data.queues[domain]
  if domain_config then
    for k, v in pairs(domain_config) do
      if allow_all or is_queue_config_option(k, v) then
        params[k] = v
      end
    end
  end

  local tenant_definition = data.tenants[tenant]
  if tenant_definition then
    for k, v in pairs(tenant_definition) do
      if allow_all or is_queue_config_option(k, v) then
        params[k] = v
      end
    end
  end

  if domain_config then
    local tenant_config = domain_config.tenants[tenant]
    if tenant_config then
      for k, v in pairs(tenant_config) do
        if allow_all or is_queue_config_option(k, v) then
          params[k] = v
        end
      end

      local campaign = tenant_config.campaigns[campaign]

      if campaign then
        for k, v in pairs(campaign) do
          if allow_all or is_queue_config_option(k, v) then
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

local function resolve_overall_throttle_specs(
  data,
  tenant_name,
  campaign_name
)
  local results = {}
  local tenant = data.tenants[tenant_name]
  if tenant then
    local rate = tenant.overall_max_message_rate
    if rate then
      results.tenant = rate
    end

    local campaign = tenant.campaigns[campaign_name]
    if campaign then
      results.campaign = campaign.overall_max_message_rate
    end
  end

  return results
end

local function apply_impl(msg, data)
  if data.scheduling_header then
    msg:import_scheduling_header(data.scheduling_header, true)
  end
  if data.campaign_header then
    local campaign = msg:get_first_named_header_value(data.campaign_header)
    if campaign then
      msg:set_meta('campaign', campaign)
      if data.remove_campaign_header then
        msg:remove_all_named_headers(data.campaign_header)
      end
    end
  end
  local tenant = nil
  local tenant_source = nil
  if data.tenant_header then
    tenant = msg:get_first_named_header_value(data.tenant_header)
    if tenant then
      tenant_source = string.format("'%s' header", data.tenant_header)
      if data.remove_tenant_header then
        msg:remove_all_named_headers(data.tenant_header)
      end
    end
  end
  if not tenant and data.default_tenant then
    tenant = data.default_tenant
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

  local recip = msg:recipient()
  if recip then
    local composed = resolve_config(
      data,
      recip.domain,
      msg:get_meta 'tenant',
      msg:get_meta 'campaign',
      true -- allow non-queue-config options
    )
    if composed then
      local routing_domain = composed.routing_domain
      if routing_domain then
        msg:set_meta('routing_domain', routing_domain)
      end
    end
  end
end

function mod:setup(file_names)
  return self:setup_with_options {
    skip_queue_config_hook = false,
    file_names = file_names,
  }
end

function mod:setup_with_options(options)
  if mod.CONFIGURED then
    error 'queues module has already been configured'
  end

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

  mod.CONFIGURED = {
    options = options,
    get_data = function()
      return load_queue_config(options.file_names, true)
    end,
    resolve_config = helper.resolve_config,
  }

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

  -- Apply any overall_max_message_rate option that may have been
  -- specified for the tenant or its campaign(s)
  kumo.on('throttle_insert_ready_queue', function(msg)
    local tenant_name = msg:get_meta 'tenant'
    local campaign_name = msg:get_meta 'campaign'

    if tenant_name or campaign_name then
      local data = cached_load_data(options.file_names)
      local throttles =
        resolve_overall_throttle_specs(data, tenant_name, campaign_name)

      if throttles.tenant then
        local throttle = kumo.make_throttle(
          string.format(
            'queue-helper-tenant-overall_max_message_rate-%s',
            tenant_name
          ),
          throttles.tenant
        )
        if throttle:delay_message_if_throttled(msg) then
          return
        end
      end

      if throttles.campaign then
        local throttle = kumo.make_throttle(
          string.format(
            'queue-helper-tenant-campaign-overall_max_message_rate-%s-%s',
            tenant_name,
            campaign_name
          ),
          throttles.tenant
        )
        if throttle:delay_message_if_throttled(msg) then
          return
        end
      end
    end
  end)

  function helper:apply(msg)
    local data = cached_load_data(options.file_names)
    apply_impl(msg, data)
  end

  return helper
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local data = mod.CONFIGURED.get_data()
  -- print(kumo.json_encode_pretty(data))
  local failed = false

  function show_context()
    if failed then
      return
    end
    failed = true
    kumo.validation_failed()
    print 'Issues found in the combined set of queue files:'
    for _, file_name in ipairs(mod.CONFIGURED.options.file_names) do
      if type(file_name) == 'table' then
        print ' - (inline table)'
      else
        print(string.format(' - %s', file_name))
      end
    end
  end

  local sources = require 'policy-extras.sources'
  if sources.CONFIGURED then
    local source_data = sources.CONFIGURED.get_data()

    for tenant, tenant_data in pairs(data.tenants) do
      if tenant_data.egress_pool then
        if not source_data.pools[tenant_data.egress_pool] then
          show_context()
          print(
            string.format(
              "tenant '%s' uses pool '%s' which is not present in your sources helper data",
              tenant,
              tenant_data.egress_pool
            )
          )
        end
      end
    end

    for domain, domain_data in pairs(data.queues) do
      if domain_data.egress_pool then
        if not source_data.pools[domain_data.egress_pool] then
          show_context()
          print(
            string.format(
              "domain '%s' uses pool '%s' which is not present in your sources helper data",
              domain,
              domain_data.egress_pool
            )
          )
        end
      end
    end
  end

  local function stack_contains_src(stack, src)
    for _, frame in ipairs(stack) do
      if utils.starts_with(frame, src) then
        return true
      end
    end
    return false
  end

  local reg = kumo.get_event_registrars 'get_queue_config'
  local my_source = kumo.traceback(1)[1].short_src .. ':'
  local my_reg = nil
  for idx, stack in ipairs(reg) do
    if stack_contains_src(stack, my_source) then
      my_reg = idx
      break
    end
  end

  if my_reg ~= #reg then
    -- Don't use show_context() here, as this error is independent of
    -- the contents of the queue data files and instead about interactions
    -- between modules
    kumo.validation_failed()

    print [[
queue.lua is in use, but it is not the last module to register for the get_queue_config event.
This can cause issues with routing/scheduling, especially if you have a [queue.default]
block defined in your queue data.

Here are the locations where each of the get_queue_config events are
registered:
]]

    for idx, stack in ipairs(reg) do
      print(string.format('%d:', idx))
      for _, frame in ipairs(stack) do
        print(string.format('    %s', frame))
      end
    end

    print [[

You should adjust the initialization order so that queue.lua is last.
]]
  end
end)

function mod:test()
  local base_data = [=[
default_tenant = 'mytenant'

[queue.default]
max_age = '24 hours'

[queue.'my.own.hostname']
routing_domain = '[10.0.0.1]'

[tenant.'mytenant']
egress_pool = 'tpool'
overall_max_message_rate = "100/s"

[tenant.'mytenant'.campaigns.'mycampaign']
overall_max_message_rate = "50/s"

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

  utils.assert_eq(resolve_overall_throttle_specs(data, nil, nil), {})
  utils.assert_eq(
    resolve_overall_throttle_specs(data, 'some-tenant', nil),
    {}
  )
  utils.assert_eq(
    resolve_overall_throttle_specs(data, 'mytenant', nil),
    { tenant = '100/s' }
  )
  utils.assert_eq(
    resolve_overall_throttle_specs(data, 'mytenant', 'mycampaign'),
    { tenant = '100/s', campaign = '50/s' }
  )

  local function new_msg(recip)
    return kumo.make_message(
      'sender@example.com',
      recip,
      'Subject: hello\r\n\r\nHi'
    )
  end

  local msg = new_msg 'recip@example.com'
  apply_impl(msg, data)
  utils.assert_eq(msg:get_meta 'tenant', 'mytenant')

  local msg = new_msg 'recip@my.own.hostname'
  apply_impl(msg, data)
  utils.assert_eq(msg:get_meta 'tenant', 'mytenant')
  utils.assert_eq(msg:get_meta 'routing_domain', '[10.0.0.1]')
end

return mod
