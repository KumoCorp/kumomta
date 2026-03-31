-- Policy file for testing mail delivery through a SOCKS5 proxy
local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local PROXY_SERVER = os.getenv 'KUMO_PROXY_SERVER_ADDRESS'
local PROXY_USERNAME = os.getenv 'KUMO_PROXY_USERNAME'
local PROXY_PASSWORD = os.getenv 'KUMO_PROXY_PASSWORD'

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

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
  }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local pool = 'proxy_pool'
  if domain == 'unplumbed.example.com' then
    pool = 'unplumbed_pool'
  end

  return kumo.make_queue_config {
    protocol = {
      smtp = {
        mx_list = { 'localhost:' .. SINK_PORT },
      },
    },
    egress_pool = pool,
  }
end)

local POOLS = {
  proxy_pool = {
    source = 'proxy_pool',
    source_address = '127.0.0.1',
  },
  unplumbed_pool = {
    source = 'unplumbed_pool',
    source_address = '9.9.9.9',
  },
}

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = POOLS[pool_name].source },
    },
  }
end)

kumo.on('get_egress_source', function(source_name)
  local params = {
    name = source_name,
    socks5_proxy_server = PROXY_SERVER,
    socks5_proxy_source_address = POOLS[source_name].source_address,
  }

  -- Add authentication if provided
  if PROXY_USERNAME and PROXY_PASSWORD then
    params.socks5_proxy_username = PROXY_USERNAME
    params.socks5_proxy_password = {
      key_data = PROXY_PASSWORD,
    }
  end

  return kumo.make_egress_source(params)
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  return kumo.make_egress_path {
    enable_tls = 'Disabled',
    ip_lookup_strategy = 'Ipv4Only',
    prohibited_hosts = {},
  }
end)
