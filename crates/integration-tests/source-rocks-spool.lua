-- Minimal source policy used by spool_write_stopped tests.
--
-- The goal is to exercise the rocksdb spool backend deterministically
-- in the configuration an operator would actually deploy:
-- paranoid_checks left at its default (off), so rocksdb's normal
-- silent-drop behavior on a corrupt SST applies.  The test relies on
-- the fact that a corruption discovered during compaction still
-- increments rocksdb.background-errors even when paranoid_checks is
-- off, which is what the load-shedding gate latches on.
--
-- error_latch_duration is shortened so the gate engages within test
-- timescales rather than 15 seconds of real time.  All other rocksdb
-- tuning is just to drive the database through its flush/compact
-- lifecycle in a handful of writes.
--
-- example.com is routed to the maildir sink so that any messages
-- accepted before the load-shedding gate latches do not escape the
-- test sandbox.
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

kumo.on('init', function()
  -- The default paths for these point under /var/spool and would
  -- otherwise mutate (or fail to access) real production locations.
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }
  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
  }

  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
    -- Pinning the hostname removes the only source of variance in
    -- the SMTP responses, so the test can assert on exact strings
    -- rather than substrings.
    hostname = 'kumo.test',
  }

  kumo.start_http_listener {
    listen = '127.0.0.1:0',
  }

  local rocks_params = {
    -- Tiny write_buffer_size forces fast SST production so that the
    -- test can drive rocksdb through its flush/compact lifecycle in
    -- a handful of writes.  A test that wants freshly-written data to
    -- stay in the WAL (rather than being flushed to an SST) raises
    -- this via KUMOD_ROCKS_WRITE_BUFFER_SIZE.
    write_buffer_size = tonumber(os.getenv 'KUMOD_ROCKS_WRITE_BUFFER_SIZE')
      or 4096,
    -- Short window so the gate latches in test time rather than in
    -- 15 seconds of real time.  Combined with the 5-second
    -- metrics_monitor tick, the gate engages within ~5-10s of the
    -- corruption being introduced.
    error_latch_duration = '2s',
  }

  kumo.define_spool {
    name = 'data',
    path = TEST_DIR .. '/data-spool',
    kind = 'RocksDB',
    rocks_params = rocks_params,
  }
  kumo.define_spool {
    name = 'meta',
    path = TEST_DIR .. '/meta-spool',
    kind = 'RocksDB',
    rocks_params = rocks_params,
  }
end)

kumo.on('smtp_server_message_received', function(msg)
  -- no-op; we accept whatever the client sends
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  -- Route example.com to the local maildir sink so any messages
  -- accepted by the test do not leak to the public internet.
  return kumo.make_queue_config {
    protocol = {
      smtp = {
        mx_list = { 'localhost:' .. SINK_PORT },
      },
    },
  }
end)

kumo.on('get_egress_path_config', function(domain, source_name, _site_name)
  -- Override the default prohibited_hosts so the sink running on
  -- 127.0.0.1 is reachable.  enable_tls = OpportunisticInsecure
  -- skips strict cert validation so the dispatcher gets far enough
  -- to load the message data from spool, which is the code path
  -- this test needs to exercise to surface a missing SST.
  return kumo.make_egress_path {
    prohibited_hosts = {},
    enable_tls = 'OpportunisticInsecure',
  }
end)
