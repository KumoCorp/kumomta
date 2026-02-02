local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local BACKUP_HOST = 'http://127.0.0.1:' .. os.getenv 'KUMOD_HTTP_SINK_PORT'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
  }

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

kumo.on(
  'requeue_message',
  function(msg, smtp_response, insert_context, increment_attempts, delay)
    print(
      string.format(
        'requeue_message called inc=%s ctx=%s delay=%s',
        increment_attempts,
        kumo.serde.json_encode(insert_context),
        delay
      )
    )
    kumo.xfer.xfer_in_requeue(
      msg,
      BACKUP_HOST,
      insert_context,
      increment_attempts,
      delay,
      'reroute to backup infra'
    )
  end
)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local protocol = {
    -- Redirect traffic to the sink
    smtp = {
      mx_list = { 'localhost:' .. SINK_PORT },
    },
  }

  return kumo.make_queue_config {
    protocol = protocol,
    -- Use a short interval because the xfer will increment the attempts
    -- and respect the delay from the transfail and we don't want to wait 20 minutes
    -- in the test
    max_retry_interval = '5s',
    retry_interval = '5s',
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  -- Allow sending to a sink
  local params = {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    -- Skip IPv6 addresses that come back for eg: localhost.
    -- For the most part the integration tests don't care about this,
    -- but the disconnect_XXX tests do make some assertions on the
    -- ordering, and in particular, disconnect_terminate_ok will be
    -- unhappy if the second address in its MX plan is unroutable IPv6.
    skip_hosts = { '::/0' },
  }
  kumo.log_warn('get_egress_path_config *******************', domain)

  return kumo.make_egress_path(params)
end)
