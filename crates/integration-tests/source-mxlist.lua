local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

log_hooks = require 'policy-extras.log_hooks'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }

  local relay_hosts = { '0.0.0.0/0' }

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
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

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
    log_oob = true,
    log_arf = true,
  }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  kumo.log_warn('get_queue_config', domain, tenant, campaign, routing_domain)

  local sink = 'localhost:' .. SINK_PORT

  local host_map = kumo.serde.json_load(TEST_DIR .. '/queue-data.json')
  local mx = host_map[domain] or { sink }

  local protocol = {
    -- Redirect traffic to the sink
    smtp = {
      mx_list = mx,
    },
  }

  return kumo.make_queue_config {
    protocol = protocol,
    refresh_interval = '3s',
    refresh_strategy = 'Ttl',
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
    idle_timeout = '2s',
  }

  kumo.log_warn('get_egress_path_config *******************', domain)

  return kumo.make_egress_path(params)
end)
