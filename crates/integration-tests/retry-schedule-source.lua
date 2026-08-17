-- Source policy for the retry_schedule_nxdomain integration test.
--
-- The test drives a recipient whose domain never resolves, so that each
-- delivery attempt fails during MX resolution and the message is rescheduled.
-- A test resolver is used instead of live DNS so the NXDOMAIN answer is
-- deterministic and returned immediately.
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'

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

  -- The recipient domain is not present in any configured zone, so resolving
  -- it yields NXDOMAIN immediately rather than consulting live DNS. The zone
  -- below only exists to give the resolver a valid, non-empty configuration.
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN example.com.
placeholder 600 A 127.0.0.1
]],
  }
end)

kumo.on(
  'get_queue_config',
  function(_domain, _tenant, _campaign, _routing_domain)
    return kumo.make_queue_config {
      retry_interval = os.getenv 'KUMOD_RETRY_INTERVAL',
      strategy = os.getenv 'KUMOD_QUEUE_STRATEGY',
    }
  end
)
