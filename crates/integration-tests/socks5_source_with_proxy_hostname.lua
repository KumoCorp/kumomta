-- Policy file for testing mail delivery through a SOCKS5 proxy that
-- is identified by a DNS host name. A test resolver maps the hostname
-- `socks5.proxy.test` to 127.0.0.1, so resolution exercises the new
-- hostname code path in EgressSource::connect_to.
local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local PROXY_SERVER = os.getenv 'KUMO_PROXY_SERVER_ADDRESS'

-- Extract the port from the proxy server address (assumed to be on 127.0.0.1).
local PROXY_PORT = PROXY_SERVER:match ':(%d+)$'
assert(PROXY_PORT, 'expected KUMO_PROXY_SERVER_ADDRESS to be host:port')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  kumo.dns.configure_test_resolver {
    [[
$ORIGIN socks5.proxy.test.
@ 30 IN A 127.0.0.1
    ]],
  }

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

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
  }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    protocol = {
      smtp = {
        mx_list = { '127.0.0.1:' .. SINK_PORT },
      },
    },
    egress_pool = 'proxy_pool',
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = 'proxy_pool' },
    },
  }
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    socks5_proxy_server = 'socks5.proxy.test:' .. PROXY_PORT,
    socks5_proxy_source_address = '127.0.0.1',
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  return kumo.make_egress_path {
    enable_tls = 'Disabled',
    ip_lookup_strategy = 'Ipv4Only',
    prohibited_hosts = {},
  }
end)
