local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  kumo.start_http_listener {
    listen = '127.0.0.1:0',
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

-- Exercise the exact production Lua bindings (msg:from_header(), which is
-- Message::get_address_header("From") under the hood, and the equivalent
-- for Reply-To) right after HTTP injection builds the message, so this is
-- the real code path a policy script hits, not just a Rust-side re-check.
kumo.on('http_message_generated', function(msg)
  local ok, err = pcall(function()
    return msg:from_header()
  end)
  if not ok then
    error('msg:from_header() failed: ' .. tostring(err))
  end

  local ok2, err2 = pcall(function()
    return msg:get_address_header 'Reply-To'
  end)
  if not ok2 then
    error('msg:get_address_header("Reply-To") failed: ' .. tostring(err2))
  end
end)

kumo.on(
  'get_queue_config',
  function(domain, _tenant, _campaign, _routing_domain)
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
