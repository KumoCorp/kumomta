-- This config acts as a sink that capture all received mail into a maildir
local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path
local utils = require 'policy-extras.policy_utils'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
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

  zones = {}
  zones[1] = '$ORIGIN localhost.\n'..
    '@        600 TXT "v=spf1 all"\n'..
    'denied       TXT "v=spf1 -all"\n'..
    'allowed      TXT "v=spf1 +all"\n'
  kumo.dns.configure_test_resolver(zones)
end)

kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', 'maildir')
end)

kumo.on('get_queue_config', function(_domain, _tenant, _campaign)
  return kumo.make_queue_config {
    protocol = {
      maildir_path = TEST_DIR .. '/maildir',
    },
  }
end)

kumo.on('smtp_server_ehlo', function(domain, conn_meta)
  local result = kumo.spf.check_host(domain, conn_meta, nil)
  print('SPF-ehlo', kumo.json_encode_pretty(result))
  if result.disposition == 'fail' then
    kumo.reject(550, '5.7.1 SPF MAIL FROM check failed')
  end
end)

kumo.on('smtp_server_mail_from', function(sender, conn_meta)
  local result = kumo.spf.check_host(sender.domain, conn_meta, tostring(sender))
  print('SPF-mail-from', kumo.json_encode_pretty(result))
  if result.disposition == 'fail' then
    kumo.reject(550, '5.7.1 SPF MAIL FROM check failed')
  end
end)
