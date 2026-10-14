local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('pre_init', function()
  -- Set the limit to 1 to force a second, overlapping request to be
  -- rejected deterministically.
  kumo.set_httpinject_concurrency_limit(1)
end)

kumo.on('init', function()
  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.define_spool {
    kind = 'RocksDB',
    name = 'data',
    path = TEST_DIR .. '/data-spool',
  }

  kumo.define_spool {
    kind = 'RocksDB',
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
  }
end)

kumo.on('http_message_generated', function(_msg)
  -- Keep each request in flight long enough for the test to overlap a second
  -- request against it while the first still holds its concurrency slot.
  kumo.time.sleep(2.0)
end)

kumo.on(
  'get_queue_config',
  function(_domain, _tenant, _campaign, _routing_domain)
    return kumo.make_queue_config {
      protocol = {
        smtp = {
          mx_list = { 'localhost:' .. SINK_PORT },
        },
      },
    }
  end
)

kumo.on('get_egress_path_config', function(_domain, _source_name, _site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
  }
end)
