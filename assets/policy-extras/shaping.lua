local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
local sources = require 'policy-extras.sources'

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

local function load_shaping_data(file_names, validation_options)
  -- print('Loading shaping data from ', kumo.json_encode(file_names))
  local result = kumo.shaping.load(file_names, validation_options)
  local errors = result:get_errors()
  for _, err in ipairs(errors) do
    kumo.log_error(err)
  end
  local warnings = result:get_warnings()
  for _, warn in ipairs(warnings) do
    kumo.log_warn(warn)
  end
  -- print('Computed', kumo.json_encode_pretty(result))
  return result
end

-- Log records that are not interesting to TSA or for automation
-- purposes: these represent messages coming into the system,
-- or movement through internal queues, rather than interactions
-- with a destination system
local UNINTERESTING_LOG_RECORD_TYPES = {
  Reception = true,
  AdminRebind = true,
  DeferredInjectionRebind = true,
  Delayed = true,
}

local function should_enq(publish, msg, hook_name, options)
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

  if UNINTERESTING_LOG_RECORD_TYPES[log_record.type] then
    return false
  end

  if options.pre_filter then
    -- Only bother shipping the log record over if it matches
    -- a TSA rule, so that we can avoid increasing IO pressure
    -- for the records that don't match
    local status, result = pcall(function()
      local shaping = mod.CONFIGURED.load_shaping_data()
      return #shaping:match_rules(log_record) > 0
    end)
    if not status then
      return false
    end
    if not result then
      return false
    end
  end

  -- It was not destined to any of our hooks, so we can safely
  -- queue this one without triggering a cycle
  msg:set_meta('queue', hook_name)
  return true
end

local function construct_publisher(publish, domain, options)
  local connection = {}
  local client = kumo.http.build_client {
    timeout = options.publish_timeout or '1 minute',
    pool_idle_timeout = options.publish_pool_idle_timeout or '90 seconds',
    connection_verbose = options.publish_connection_verbose,
  }
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

local function get_queue_cfg(options, publish, domain, tenant, campaign)
  for _, data in pairs(publish) do
    if data.hook_name == domain then
      local params = {
        retry_interval = '1m',
        max_retry_interval = '20m',
      }
      utils.merge_into(options.tsa_queue_config, params)
      params.protocol = {
        custom_lua = {
          constructor = data.constructor,
        },
      }
      return kumo.make_queue_config(params)
    end
  end
end

local function apply_ready_q_suspension(items)
  if #items == 0 then
    return
  end
  kumo.log_debug('apply_ready_q_suspension', #items)
  for _, item in ipairs(items) do
    local reason =
      string.format('%s (rule_hash=%s)', item.reason, item.rule_hash)

    kumo.api.admin.suspend_ready_q.suspend {
      name = item.site_name,
      reason = reason,
      expires = item.expires,
    }
  end
  kumo.log_debug('applied ready_q_suspensions', #items)
end

local function apply_sched_q_suspension(items)
  if #items == 0 then
    return
  end
  kumo.log_debug('apply_sched_q_suspension', #items)
  for _, item in ipairs(items) do
    local reason =
      string.format('%s (rule_hash=%s)', item.reason, item.rule_hash)

    kumo.api.admin.suspend.suspend {
      campaign = item.campaign,
      domain = item.domain,
      tenant = item.tenant,
      reason = reason,
      expires = item.expires,
    }
  end
  kumo.log_debug('applied sched_q_suspensions', #items)
end

local function apply_sched_q_bounce(items)
  if #items == 0 then
    return
  end
  kumo.log_debug('apply_sched_q_bounce', #items)
  for _, item in ipairs(items) do
    local reason =
      string.format('%s (rule_hash=%s)', item.reason, item.rule_hash)

    kumo.api.admin.bounce.bounce {
      campaign = item.campaign,
      domain = item.domain,
      tenant = item.tenant,
      reason = reason,
      expires = item.expires,
    }
  end
  kumo.log_debug('applied sched_q_bounce', #items)
end

kumo.on('kumo.tsa.config.monitor', function(args)
  local last_hash = ''
  kumo.log_info 'TSA config monitor running'
  while true do
    local status, err = pcall(function()
      local shaping = mod.CONFIGURED.load_shaping_data()
      local hash = shaping:hash()
      if last_hash ~= hash then
        kumo.log_info 'TSA config changed'
        last_hash = hash
        kumo.bump_config_epoch()
      end
    end)
    if not status then
      kumo.log_error('TSA Error, will retry in 30 seconds', status, err)
    end
    -- load_shaping_data is a memoized function that will return
    -- a cached reference to the shaping data, but if TSA is producing
    -- many records then lua will accumulate a variety of those references.
    -- For some sites, the TSA data can be very large.
    -- Ordinarily, lua will gc based on its perceived memory usage
    -- but the memory consumed by the shaping data is largely
    -- invisible to lua because it is an indirect Arc<> to the
    -- bulk of the data.
    -- So we explicitly collect garbage now to encourage lua
    -- to drop the un-referenced shaping data before we go
    -- to sleep and keep things better trimmed.
    collectgarbage()
    kumo.sleep(30)
  end
end)

local function process_tsa_events(url)
  -- Generate the websocket URL from the user-provided HTTP URL
  local ws_url = url:gsub('^http', 'ws')

  -- First let's try the new generic event endpoint URL
  local event_endpoint = string.format('%s/subscribe_event_v1', ws_url)

  local status, err_or_stream, response =
    pcall(kumo.http.connect_websocket, event_endpoint)
  if status then
    -- Daemon supports the new endpoint

    -- Loop and consume all suspensions from the host; the initial
    -- connection will pre-populate the stream with any current
    -- suspensions, and then will later deliver any subsequently
    -- generated suspension events in realtime.
    while true do
      local batch = err_or_stream:recv_batch '3s'
      local ready_q_sus = {}
      local sched_q_sus = {}
      local sched_q_bounce = {}
      for _, item in ipairs(batch) do
        local data = kumo.json_parse(item)
        if data.ReadyQSuspension then
          table.insert(ready_q_sus, data.ReadyQSuspension)
        elseif data.SchedQSuspension then
          table.insert(sched_q_sus, data.SchedQSuspension)
        elseif data.SchedQBounce then
          table.insert(sched_q_bounce, data.SchedQBounce)
        else
          kumo.log_error(
            string.format(
              'Received unsupported record type %s from TSA. Do you need to upgrade kumod on this instance?',
              kumo.serde.json_encode(data)
            )
          )
        end
      end
      apply_ready_q_suspension(ready_q_sus)
      apply_sched_q_suspension(sched_q_sus)
      apply_sched_q_bounce(sched_q_bounce)
    end
  elseif tostring(err_or_stream):find 'HTTP error: 404 Not Found' then
    -- Daemon is up, but the endpoint is not supported.
    -- Fall back to the legacy endpoint
    local endpoint = string.format('%s/subscribe_suspension_v1', ws_url)

    kumo.log_warn(
      string.format(
        "NOTE: Your TSA daemon doesn't support %s, falling back to %s. Please upgrade and restart your TSA daemon to enable full functionality!",
        event_endpoint,
        endpoint
      )
    )

    local stream, response = kumo.http.connect_websocket(endpoint)

    -- Loop and consume all suspensions from the host; the initial
    -- connection will pre-populate the stream with any current
    -- suspensions, and then will later deliver any subsequently
    -- generated suspension events in realtime.
    while true do
      local data = kumo.json_parse(stream:recv())
      if data.ReadyQ then
        apply_ready_q_suspension { data.ReadyQ }
      elseif data.SchedQ then
        apply_sched_q_suspension { data.SchedQ }
      end
    end
  else
    -- Some other error
    error(
      string.format(
        'Request to %s failed: %s',
        event_endpoint,
        tostring(err_or_stream)
      )
    )
  end
end

kumo.on('kumo.tsa.suspension.subscriber', function(args)
  local url = args[1]

  -- If we encounter an error (likely cause: tsa-daemon restarting),
  -- then we'll try again after a short sleep
  while true do
    local status, err = pcall(process_tsa_events, url)
    kumo.log_error('TSA Error, will retry in 30 seconds', status, err)
    kumo.sleep(30)
  end
end)

--[[
local shaper = shaping:setup_with_automation {
  publish = {"http://10.0.0.1:8008"},
  subscribe = {"http://10.0.0.1:8008"},
  -- this needs to list any files that hold your custom shaping rules; should match
  -- the additional files beyond /opt/kumomta/share/policy-extras/shaping.toml in your
  -- tsa config
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },

  -- optional; override the queue config for talking to the TSA daemon.
  -- These are the default values.
  tsa_queue_config = {
    retry_interval = '1m',
    max_retry_interval = '20m',
  },

  -- Optional back_pressure value to pass through to the internal
  -- configure_log_hook call that is used for routing logs to TSA
  back_pressure = 256000,

  -- optional; specify the validation options to use when loading
  -- the shaping data in the live service
  load_validation_options = {
        aliased_site = 'Ignore',
        dns_fail = 'Ignore',
        local_load = 'Error',
        null_mx = 'Ignore',
        provider_overlap = 'Ignore',
        remote_load = 'Ignore',
        skip_remote = false,
        http_timeout = '5s', -- the timeout for requests to tsa daemon
  },

  -- optional; specify the validation options to use in --validate
  -- mode. You might consider making these more strict than the
  -- regular load_validation_options.
  validation_options = {
        aliased_site = 'Warn',
        dns_fail = 'Warn',
        local_load = 'Error',
        null_mx = 'Warn',
        provider_overlap = 'Warn',
        remote_load = 'Ignore',
        skip_remote = true,
  },
}

kumo.on('init', function()
  shaper.setup_publish()
end)

kumo.on('get_egress_path_config', shaper.get_egress_path_config)
]]
function mod:setup_with_automation(options)
  if mod.CONFIGURED then
    error 'shaping module has already been configured'
  end

  if options.pre_filter == nil then
    options.pre_filter = true
  end

  local cached_load_data = kumo.memoize(load_shaping_data, {
    name = 'shaping_data',
    ttl = options.cache_ttl or '1 minute',
    capacity = 4,
    invalidate_with_epoch = true,
  })

  local file_names = {}
  if not options.no_default_files then
    table.insert(file_names, '/opt/kumomta/share/policy-extras/shaping.toml')
  end

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
  for _, destination in ipairs(options.publish or {}) do
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
      return construct_publisher(publish[hook_name], domain, options)
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
          -- Likewise, rejections don't make sense to pass to TSA
          Rejection = {
            enable = false,
          },
        },
        back_pressure = options.back_pressure,
      }
    end

    if options.subscribe then
      for _, url in ipairs(options.subscribe) do
        kumo.spawn_task {
          event_name = 'kumo.tsa.suspension.subscriber',
          args = { url },
        }
      end
      kumo.spawn_task {
        event_name = 'kumo.tsa.config.monitor',
        args = {},
      }
    end
  end

  local function cached_load_shaping_data()
    return cached_load_data(file_names, options.load_validation_options)
  end

  local function get_egress_path_config(
    domain,
    egress_source,
    site_name,
    skip_make
  )
    local data = cached_load_shaping_data()
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

    if not params.refresh_strategy then
      params.refresh_strategy = 'Epoch'
    end

    if skip_make then
      -- For test harness purposes, we can set skip_make to true so that
      -- we can manipulate the params before constructing an egress path
      return params
    end
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
      return get_queue_cfg(options, publish, domain, tenant, campaign)
    end
  )

  kumo.on('should_enqueue_log_record', function(msg, hook_name)
    return should_enq(publish, msg, hook_name, options)
  end)

  mod.CONFIGURED = {
    _file_names = file_names,
    _options = options,

    load_shaping_data = cached_load_shaping_data,

    get_egress_path_config = get_egress_path_config,
    setup_publish = setup_publish,
  }

  return mod.CONFIGURED
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local result = kumo.shaping.load(
    mod.CONFIGURED._file_names,
    mod.CONFIGURED._options.validation_options
      or {
        aliased_site = 'Warn',
        dns_fail = 'Warn',
        local_load = 'Error',
        null_mx = 'Warn',
        provider_overlap = 'Warn',
        remote_load = 'Ignore',
        skip_remote = true,
      }
  )
  local warnings = result:get_warnings()
  local errors = result:get_errors()
  local did_header = false

  function show_context()
    if did_header then
      return
    end
    did_header = true
    print 'Issues found in the combined set of shaping files:'
    for _, file_name in ipairs(mod.CONFIGURED._file_names) do
      print(string.format(' - %s', file_name))
    end
  end

  if #errors > 0 then
    show_context()
    kumo.validation_failed()
    for _, err in ipairs(errors) do
      print('ERROR: ' .. err)
    end
  end

  if #warnings > 0 then
    show_context()
    for _, warn in ipairs(warnings) do
      print('WARNING: ' .. warn)
    end
  end

  if sources.CONFIGURED then
    local source_data = sources.CONFIGURED.get_data()
    local refd_sources = result:get_referenced_sources()
    for source, refs in pairs(refd_sources) do
      if source == 'my source name' and refs[1] == 'domain:example.com' then
        -- Ignore sample data from default shaping.toml
      else
        if not source_data.sources[source] then
          show_context()
          kumo.validation_failed()
          print(
            string.format(
              "Source '%s' is not present in your sources helper data. Referenced by %s",
              source,
              table.concat(refs, ', ')
            )
          )
        end
      end
    end
  end
end)

return mod
