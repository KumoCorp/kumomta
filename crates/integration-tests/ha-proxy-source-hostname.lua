local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  -- Map proxy.test to localhost so the hostname-based ha_proxy_server
  -- exercises the DNS resolution path in EgressSource::connect_to.
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN proxy.test.
@ 30 IN A 127.0.0.1
    ]],
  }

  local relay_hosts = { '0.0.0.0/0' }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
    deferred_queue = false,
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
  return kumo.make_queue_config {
    protocol = {
      smtp = {
        mx_list = { '127.0.0.1:' .. SINK_PORT },
      },
    },
    egress_pool = 'default',
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = 'default' },
    },
  }
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    ha_proxy_source_address = '127.0.0.1',
    ha_proxy_server = 'proxy.test:' .. SINK_PORT,
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  local params = {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    opportunistic_tls_reconnect_on_failed_handshake = false,
    ip_lookup_strategy = 'Ipv4Only',
  }
  return kumo.make_egress_path(params)
end)
