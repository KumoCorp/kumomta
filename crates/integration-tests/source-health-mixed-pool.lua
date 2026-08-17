-- Pool containing one healthy source and one unplumbed source. The
-- broken source has an Immediate suspend_when_unplumbed rule, so once
-- selection routes a message through it and fails, it is auto-suspended
-- and subsequent selections skip it. Mail must continue to flow via
-- the healthy source.
local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
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
  return kumo.make_listener_domain { relay_to = true }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    protocol = {
      smtp = {
        mx_list = { '127.0.0.1:' .. SINK_PORT },
      },
    },
    egress_pool = 'mixed',
    -- Short retry interval so transiently-failed messages get reattempted
    -- (against the surviving healthy source) within the test's time budget.
    retry_interval = '2s',
    max_retry_interval = '2s',
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = 'bad' },
      { name = 'good' },
    },
  }
end)

kumo.on('get_egress_source', function(source_name)
  if source_name == 'bad' then
    return kumo.make_egress_source {
      name = source_name,
      source_address = '9.9.9.9',
      suspend_when_unplumbed = {
        duration = '60s',
      },
    }
  end
  return kumo.make_egress_source {
    name = source_name,
    source_address = '127.0.0.1',
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    ip_lookup_strategy = 'Ipv4Only',
  }
end)
