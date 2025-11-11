local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

log_hooks = require 'policy-extras.log_hooks'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local WEBHOOK_PORT = os.getenv 'KUMOD_WEBHOOK_PORT'
local AMQPHOOK_URL = os.getenv 'KUMOD_AMQPHOOK_URL'
local AMQP_HOST_PORT = os.getenv 'KUMOD_AMQP_HOST_PORT'
local LISTENER_MAP = os.getenv 'KUMOD_LISTENER_DOMAIN_MAP'
local DEFERRED_SMTP_SERVER_MSG_INJECT =
  os.getenv 'KUMOD_DEFERRED_SMTP_SERVER_MSG_INJECT'
local BATCH_HANDLING = (os.getenv 'KUMOD_BATCH_HANDLING') or 'BifurcateAlways'
local MAX_RECIPIENTS_PER_BATCH = (os.getenv 'KUMOD_MAX_RECIPIENTS_PER_BATCH')
  or 100
local USE_SPLIT_TXN = os.getenv 'KUMOD_USE_SPLIT_TXN'

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  local relay_hosts = { '0.0.0.0/0' }
  local RELAY_HOSTS = os.getenv 'KUMOD_RELAY_HOSTS'
  if RELAY_HOSTS then
    relay_hosts = kumo.json_parse(RELAY_HOSTS)
  end

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
    deferred_queue = (DEFERRED_SMTP_SERVER_MSG_INJECT and true) or false,
    peer = {
      ['127.0.0.1'] = {
        allow_xclient = true,
      },
    },
    via = {
      ['42.42.42.42'] = {
        banner = 'what do you get when you multiply six by nine?',
      },
    },
    batch_handling = BATCH_HANDLING,
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
    headers = { 'X-*', 'Y-*', 'Subject' },
    meta = { 'xfer*' },
  }

  if WEBHOOK_PORT then
    kumo.configure_log_hook {
      name = 'webhook',
      headers = { 'Subject', 'X-*' },
    }
  end

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

if AMQPHOOK_URL then
  log_hooks:new {
    name = 'amqp',
    constructor = function(domain, tenant, campaign)
      local sender = {}
      local client = kumo.amqp.build_client(AMQPHOOK_URL)

      function sender:send(msg)
        local result = client:publish_with_timeout({
          routing_key = 'woot',
          payload = msg:get_data(),
        }, 20000)

        if result.status == 'Ack' or result.status == 'NotRequested' then
          return string.format('250 %s', kumo.json_encode(result))
        end
        -- result.status must be `Nack`; log the full result
        kumo.reject(500, kumo.json_encode(result))
      end

      function sender:close()
        client:close()
      end

      return sender
    end,
  }
elseif AMQP_HOST_PORT then
  log_hooks:new {
    name = 'amqp',
    constructor = function(domain, tenant, campaign)
      local sender = {}
      local host, port = table.unpack(kumo.string.split(AMQP_HOST_PORT, ':'))

      function sender:send(msg)
        kumo.amqp.basic_publish {
          routing_key = 'woot',
          payload = msg:get_data(),
          connection = {
            host = host,
            port = tonumber(port),
          },
        }
        return '250 ok'
      end

      return sender
    end,
  }
end

if WEBHOOK_PORT then
  local min_batch_size = tonumber(os.getenv 'KUMOD_WEBHOOK_MIN_BATCH_SIZE')
  local max_batch_size = tonumber(os.getenv 'KUMOD_WEBHOOK_MAX_BATCH_SIZE')
  local max_batch_latency = os.getenv 'KUMOD_WEBHOOK_MAX_BATCH_LATENCY'
  if max_batch_size > 1 then
    log_hooks:new {
      name = 'webhookbatch',
      batch_size = max_batch_size,
      min_batch_size = min_batch_size,
      max_batch_latency = max_batch_latency,
      constructor = function(domain, tenant, campaign)
        local sender = {}

        local client = kumo.http.build_client {}
        function sender:send_batch(messages)
          local payload = {}
          for _, msg in ipairs(messages) do
            table.insert(payload, msg:get_meta 'log_record')
          end
          print(
            string.format(
              'batch size is %d *************** min=%d max=%d latency=%s',
              #payload,
              min_batch_size,
              max_batch_size,
              max_batch_latency
            )
          )
          local response = client
            :post(
              string.format('http://127.0.0.1:%d/log-batch', WEBHOOK_PORT)
            )
            :header('Content-Type', 'application/json')
            :body(kumo.serde.json_encode(payload))
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
          kumo.reject(500, disposition)
        end

        return sender
      end,
    }
  else
    kumo.on('should_enqueue_log_record', function(msg)
      local log_record = msg:get_meta 'log_record'
      -- avoid an infinite loop caused by logging that we logged
      if log_record.queue ~= 'webhook' then
        msg:set_meta('queue', 'webhook')
        return true
      end
      return false
    end)

    kumo.on('make.webhook', function(_domain, _tenant, _campaign)
      local sender = {}
      local client = kumo.http.build_client {}
      function sender:send(message)
        local response = client
          :post(string.format('http://127.0.0.1:%d/log', WEBHOOK_PORT))
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
        kumo.reject(500, disposition)
      end
      return sender
    end)
  end
end

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if LISTENER_MAP then
    local map = kumo.json_parse(LISTENER_MAP)
    local params = map[domain]
    if params then
      return kumo.make_listener_domain(params)
    end
  end

  return kumo.make_listener_domain {
    relay_to = true,
    log_oob = true,
    log_arf = true,
  }
end)

if USE_SPLIT_TXN then
  kumo.on('smtp_server_split_transaction', function(msg, conn)
    -- This toy implementation batches by the first letter of
    -- the local part and the same domain. It is NOT an example
    -- of a valid production use case, it is merely exercising
    -- the logic to prove that it is functioning
    local batches = {}

    local keyed = {}
    for _, recip in ipairs(msg:recipient_list()) do
      local key = recip.user:sub(1, 1) .. '@' .. recip.domain:lower()
      if not keyed[key] then
        keyed[key] = {}
      end
      table.insert(keyed[key], recip)
    end

    for _, batch in pairs(keyed) do
      table.insert(batches, batch)
    end

    return batches
  end)
end

kumo.on('smtp_server_message_received', function(msg)
  local result = msg:import_scheduling_header 'X-Schedule'
  kumo.log_info('schedule result', kumo.serde.json_encode(result))
  -- This tenant header import is used by xfer.rs as a way to set
  -- the tenant metadata for the incoming message
  msg:import_x_headers { 'tenant' }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  kumo.log_warn('get_queue_config', domain, tenant, campaign, routing_domain)
  if domain == 'webhook' then
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          constructor = 'make.webhook',
        },
      },
    }
  end

  local protocol = {
    -- Redirect traffic to the sink
    smtp = {
      mx_list = { 'localhost:' .. SINK_PORT },
    },
  }

  if domain == 'two-hosts.example.com' then
    protocol = {
      -- Redirect traffic to the sink, but with two hosts in the connection plan
      smtp = {
        mx_list = {
          { name = 'localhost-1', addr = '127.0.0.1:' .. SINK_PORT },
          { name = 'localhost-2', addr = '127.0.0.1:' .. SINK_PORT },
        },
      },
    }
  end

  if domain == 'nxdomain' then
    -- this nxdomain domain is a special domain that is assumed not
    -- to resolve. It is generated by the retry_schedule integration
    -- test. for this domain, we don't want to short-circuit dns
    -- and go to the sink, because we DO want the dns resolution
    -- to successfully return nxdomain in order for the test to
    -- exercise the appropriate logic.
    protocol = nil
  end
  if domain == 'rebind.example.com' then
    -- This domain is used by rebind_port.rs.
    -- In this case, we don't want the default sink route that was
    -- established above to take effect, we want to confirm that
    -- the port number embedded in either the domain or routing_domain
    -- are used, so we use the default smtp protocol settings here
    -- and rely on assertions in rebind_port.rs to validate the
    -- behavior.
    protocol = nil
  end

  return kumo.make_queue_config {
    protocol = protocol,
    retry_interval = os.getenv 'KUMOD_RETRY_INTERVAL',
    strategy = os.getenv 'KUMOD_QUEUE_STRATEGY',
    egress_pool = os.getenv 'KUMOD_POOL_NAME',
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  if pool_name == 'warming' then
    -- coupled with source_selection_rate_pool.rs
    return kumo.make_egress_pool {
      name = pool_name,
      entries = {
        { name = 'warming_a' },
        { name = 'warming_b' },
      },
    }
  end

  error('integration-tests/source.lua: unhandled pool ' .. pool_name)
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  -- Allow sending to a sink
  local params = {
    enable_tls = os.getenv 'KUMOD_ENABLE_TLS' or 'OpportunisticInsecure',
    reconnect_strategy = os.getenv 'KUMOD_RECONNECT_STRATEGY'
      or 'ConnectNextHost',
    prohibited_hosts = {},
    opportunistic_tls_reconnect_on_failed_handshake = (
      (os.getenv 'KUMOD_OPPORTUNISTIC_TLS_RECONNECT') and true
    ) or false,
    source_selection_rate = os.getenv 'KUMOD_SOURCE_SELECTION_RATE',
    try_next_host_on_transport_error = (
      (os.getenv 'KUMOD_TRY_NEXT_HOST_ON_TRANSPORT_ERROR') and true
    ) or false,
    tls_prefer_openssl = ((os.getenv 'KUMOD_PREFER_OPENSSL') and true)
      or false,
    max_recipients_per_batch = tonumber(MAX_RECIPIENTS_PER_BATCH),
  }

  if os.getenv 'KUMOD_CLIENT_CERTIFICATE' then
    params.tls_certificate = {
      key_data = os.getenv 'KUMOD_CLIENT_CERTIFICATE',
    }
  end
  if os.getenv 'KUMOD_CLIENT_PRIVATE_KEY' then
    params.tls_private_key = {
      key_data = os.getenv 'KUMOD_CLIENT_PRIVATE_KEY',
    }
  end

  -- See if there is a source-specific rate exported to us via the environment.
  -- We assign this using additional_source_selection_rates regardless of
  -- whether we have a more generate rate specified above so that we can
  -- excercise the additional_source_selection_rates collection logic in the core.
  local source_rate_name = 'KUMOD_SOURCE_SELECTION_RATE_'
    .. source_name:upper()
  local source_rate = os.getenv(source_rate_name)
  if source_rate then
    params.additional_source_selection_rates =
      { [source_rate_name] = source_rate }
  end

  local username = os.getenv 'KUMOD_SMTP_AUTH_USERNAME'
  local password = os.getenv 'KUMOD_SMTP_AUTH_PASSWORD'

  if username and password then
    params.smtp_auth_plain_username = username
    params.smtp_auth_plain_password = {
      key_data = password,
    }
  end

  if domain == 'two-hosts.example.com' then
    params.connection_limit = 1
  end

  if domain == 'webhookbatch.log_hook' then
    -- If we allow more connections, then we can end up
    -- with batches smaller than desired because we split
    -- them among multiple connections
    params.connection_limit = 1
  end

  kumo.log_warn('get_egress_path_config *******************', domain)

  return kumo.make_egress_path(params)
end)

if os.getenv 'KUMOD_WANT_REBIND' then
  kumo.on('rebind_message', function(message, data)
    message:append_header('X-Rebound', 'woot')
    message:set_meta('queue', data.queue)
  end)
end
