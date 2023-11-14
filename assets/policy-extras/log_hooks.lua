local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
local log_hooks = require 'policy-extras.log_hooks'

-- Call this at the top level, outside of an event handler
log_hooks:new {
  name = "webhook",
  -- log_parameters are combined with the name and
  -- passed through to kumo.configure_log_hook
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
  -- queue config are passed to kumo.make_queue_config.
  -- You can use these to override the retry parameters
  -- if you wish.
  -- The defaults are shown below.
  queue_config = {
    retry_interval = "1m",
    max_retry_interval = "20m",
  },
  constructor = function(domain, tenant, campaign)
    local connection = {}
    local client = kumo.http.build_client {}
    function connection:send(message)
      local response = client
        :post('http://10.0.0.1:4242/log')
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

      -- Signal that the webhook request failed.
      -- In this case the 500 status prevents us from retrying
      -- the webhook call again, but you could be more sophisticated
      -- and analyze the disposition to determine if retrying it
      -- would be useful and generate a 400 status instead.
      -- In that case, the message we be retryed later, until
      -- it reached it expiration.
      kumo.reject(500, disposition)
    end
    return connection
  end,
}

]]
function mod:new(options)
  kumo.on('pre_init', function()
    local log_parameters = {
      name = options.name,
    }
    utils.merge_into(options.log_parameters, log_parameters)
    kumo.configure_log_hook(log_parameters)
  end)

  -- Choose a domain name with a "TLD" that will never match a
  -- legitimate TLD. This helps to avoid collision with real
  -- functioning SMTP domains
  local domain_name = string.format('%s.log_hook', options.name)
  -- Now derive a constructor event name from that
  local constructor_name = string.format('make.%s', domain_name)

  kumo.on('should_enqueue_log_record', function(msg, hook_name)
    if hook_name ~= options.name then
      -- It's not our hook
      return
    end

    local log_record = msg:get_meta 'log_record'

    -- avoid an infinite loop caused by logging that we logged that we logged...
    if log_record.reception_protocol == 'LogRecord' then
      return false
    end

    -- was some other event that we want to log via the webhook
    msg:set_meta('queue', domain_name)
    return true
  end)

  local queue_config = {
    retry_interval = '1m',
    max_retry_interval = '20m',
  }
  utils.merge_into(options.queue_config, queue_config)
  queue_config.protocol = {
    custom_lua = {
      constructor = constructor_name,
    },
  }

  kumo.on(
    'get_queue_config',
    function(domain, tenant, campaign, routing_domain)
      if domain ~= domain_name then
        -- It's not the domain associated with our hook
        return
      end

      -- Use the `make.NAME.log_hook` event to handle delivery
      -- of webhook log records
      return kumo.make_queue_config(queue_config)
    end
  )

  -- And connect up the constructor event to the user-provided constructor
  kumo.on(constructor_name, options.constructor)
end

--[[
local log_hooks = require 'policy-extras.log_hooks'

-- Call this at the top level, outside of an event handler
log_hooks:new_json {
  name = "webhook",
  -- log_parameters are combined with the name and
  -- passed through to kumo.configure_log_hook
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
  -- queue config are passed to kumo.make_queue_config.
  -- You can use these to override the retry parameters
  -- if you wish.
  -- The defaults are shown below.
  queue_config = {
    retry_interval = "1m",
    max_retry_interval = "20m",
  },
  -- The URL to POST the JSON to
  url = "http://10.0.0.1:4242/log",
}
]]
function mod:new_json(options)
  options.constructor = function(domain, tenant, campaign)
    local connection = {}
    local client = kumo.http.build_client {}
    function connection:send(message)
      local response = client
        :post(options.url)
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

      -- Signal that the webhook request failed.
      -- In this case the 500 status prevents us from retrying
      -- the webhook call again, but you could be more sophisticated
      -- and analyze the disposition to determine if retrying it
      -- would be useful and generate a 400 status instead.
      -- In that case, the message we be retryed later, until
      -- it reached it expiration.
      kumo.reject(500, disposition)
    end
    return connection
  end
  return self:new(options)
end

return mod
