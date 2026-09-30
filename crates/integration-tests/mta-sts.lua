-- Source policy for the MTA-STS integration tests.
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

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

  -- broken.example.com publishes MX records, but its (mocked) MTA-STS policy
  -- is in enforce mode and permits none of those MX hosts, making it
  -- undeliverable until the policy is corrected. good.example.com publishes a
  -- policy that covers its MX host, so delivery proceeds normally.
  -- testing.example.com publishes a testing policy for the TLS-mode tests.
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN broken.example.com.
@    600 MX 10 mail.broken.example.com.
mail 600 A  127.0.0.1
]],
    [[
$ORIGIN good.example.com.
@    600 MX 10 mail.good.example.com.
mail 600 A  127.0.0.1
]],
    [[
$ORIGIN testing.example.com.
@    600 MX 10 mail.testing.example.com.
mail 600 A  127.0.0.1
]],
  }

  if os.getenv 'KUMOD_TESTING_DANE_UNUSABLE' then
    -- Sign the zone and publish only a PKIX-EE TLSA record. DANE SMTP treats
    -- that usage as unusable, so STARTTLS is required without authentication.
    kumo.dns.configure_test_resolver {
      {
        zone = '$ORIGIN testing.example.com.\n@ 600 MX 10 mail.testing.example.com.\nmail 600 A 127.0.0.1\n_'
          .. SINK_PORT
          .. '._tcp.mail 600 TLSA 1 0 0 00\n',
        secure = true,
      },
    }
  end

  kumo.dns.configure_test_mta_sts {
    ['broken.example.com'] = [[
version: STSv1
mode: enforce
mx: allowed.example.net
max_age: 86400
]],
    ['testing.example.com'] = [[
version: STSv1
mode: testing
mx: mail.testing.example.com
max_age: 86400
]],
    ['good.example.com'] = [[
version: STSv1
mode: enforce
mx: mail.good.example.com
max_age: 86400
]],
  }
end)

kumo.on('get_queue_config', function(domain)
  -- Force real MX resolution (and thus MTA-STS evaluation) for the broken
  -- domain rather than short-circuiting to a sink.
  return kumo.make_queue_config {
    protocol = nil,
    -- Keep each TLS-mode test to one delivery attempt.
    retry_interval = os.getenv 'KUMOD_TESTING_TLS' and '1h' or '2s',
  }
end)

kumo.on('get_egress_path_config', function(domain)
  return kumo.make_egress_path {
    enable_tls = os.getenv 'KUMOD_TESTING_TLS' or 'OpportunisticInsecure',
    remember_broken_tls = os.getenv 'KUMOD_TESTING_REMEMBER_BROKEN_TLS',
    prohibited_hosts = {},
    -- Direct the resolved 127.0.0.1 MX host at the sink.
    smtp_port = SINK_PORT,
    -- Only the testing domain applies MTA-STS TLS; the enforce-policy
    -- MX-filtering tests keep an opportunistic sink hop.
    enable_mta_sts = domain == 'testing.example.com',
    enable_dane = os.getenv 'KUMOD_TESTING_DANE_UNUSABLE' ~= nil,
  }
end)
