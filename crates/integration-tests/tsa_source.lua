local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path
local shaping = require 'policy-extras.shaping'
local utils = require 'policy-extras.policy_utils'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local TSA_URL =
  string.format('http://127.0.0.1:%s', os.getenv 'KUMOD_TSA_PORT')

local shaper = shaping:setup_with_automation {
  publish = { TSA_URL },
  subscribe = { TSA_URL },
  cache_ttl = '1 second',
  no_default_files = true,
  extra_files = { 'shaping.toml' },
}

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
  shaper.setup_publish()
end)

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  return kumo.make_listener_domain {
    relay_to = true,
    log_oob = true,
    log_arf = true,
  }
end)

kumo.on('smtp_server_message_received', function(msg)
  local tenant = msg:get_first_named_header_value 'X-Tenant'
  local campaign = msg:get_first_named_header_value 'X-campaign'
  print('****** tenant', tenant, 'camp', campaign)
  msg:set_meta('tenant', tenant)
  msg:set_meta('campaign', campaign)
  msg:set_meta('routing_domain', 'localhost')
end)

kumo.on('get_egress_path_config', function(domain, egress_source, site_name)
  -- Redirect to sink port
  local skip_make = true
  local params =
    shaper.get_egress_path_config(domain, egress_source, site_name, skip_make)
  params.smtp_port = SINK_PORT
  params.prohibited_hosts = {}
  return kumo.make_egress_path(params)
end)
