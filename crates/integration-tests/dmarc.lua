local kumo = require 'kumo'
package.path = '../../assets/?.lua;' .. package.path

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

local queue_module = require 'policy-extras.queue'
local log_hooks = require 'policy-extras.log_hooks'

local queue_helper = queue_module:setup {
  {
    queues = {
      default = {
        -- Redirect traffic to the sink
        protocol = {
          smtp = {
            mx_list = { '[127.0.0.1]:' .. SINK_PORT },
          },
        },
      },
    },
  },
}

local function dmarc_generator(msg, log_record)
  print('DMARC?', kumo.serde.json_encode_pretty(log_record))
end

log_hooks:new_disposition_hook {
  name = 'dmarc_generator',
  hook = dmarc_generator,
}

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')

  local relay_hosts = { '0.0.0.0/0' }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = relay_hosts,
    deferred_queue = false,
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  kumo.dns.configure_test_resolver {
    [[
$ORIGIN 0.0.127.in-addr.arpa.
1 30 IN PTR localhost.localdomain.
2 20 IN PTR borked.localdomain.
]],
    [[
$ORIGIN localdomain.
localhost 30 IN A 127.0.0.1
]],
    [[
$ORIGIN example.com.
_dmarc 600 TXT "v=DMARC1; p=reject; aspf=s; adkim=s; rua=mailto:dmarc-feedback@example.com"
mx 3600 IN A 127.0.0.1
@ 3600 MX 10 mx.example.com.
sub 3600 IN MX 10 mx.example.com.
]],
  }

  kumo.dmarc.set_report_window_in_seconds(4)
  kumo.dmarc.start_dmarc_reporter()

  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
    headers = { 'X-*', 'Y-*', 'Subject' },
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

kumo.on('dmarc_report_generated', function(body, email)
  local msg = kumo.make_message(
    'sender@example.com',
    email,
    'From: sender@example.com\r\nSubject: DMARC Report\r\n\r\n' .. body
  )

  local ok, err = pcall(kumo.inject_message, msg)
  if not ok then
    kumo.log_error('failed to inject DMARC: ', err)
  end
end)

kumo.on('smtp_server_message_received', function(msg)
  local dkim_auth_results = msg:dkim_verify()
  if #dkim_auth_results == 0 then
    dkim_auth_results = {
      {
        method = 'dkim',
        result = 'none',
        reason = 'message was not signed',
      },
    }
  end

  msg:set_meta('received_from', '127.0.0.1:42')

  local spf_disp = kumo.spf.check_msg(msg)
  local spf_auth_result = spf_disp.result

  kumo.log_info 'DMARC checking message'
  kumo.dmarc.check_msg(msg, dkim_auth_results, nil, spf_auth_result, true, {
    org_name = 'testorg',
    email = 'dmarc-feedback@example.com',
  })

  local result = msg:import_scheduling_header 'X-Schedule'
  kumo.log_info('schedule result', kumo.serde.json_encode(result))
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  -- Allow sending to a sink
  local params = {
    enable_tls = 'OpportunisticInsecure',
    prohibited_hosts = {},
    opportunistic_tls_reconnect_on_failed_handshake = false,
  }

  kumo.log_warn('get_egress_path_config *******************', domain)

  return kumo.make_egress_path(params)
end)
