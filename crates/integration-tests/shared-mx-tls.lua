-- Shared ready queue with independently controlled recipient-domain policies.
local kumo = require 'kumo'
local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local POLICY_MAX_AGE = os.getenv 'KUMOD_SHARED_MX_POLICY_MAX_AGE' or '600'
local MESSAGE_WORK = os.getenv 'KUMOD_SHARED_MX_MESSAGE_WORK'
local MESSAGE_WORK_DIR = os.getenv 'KUMOD_SHARED_MX_MESSAGE_WORK_DIR'

local function hold_message_work()
  local entered = MESSAGE_WORK_DIR .. '/work-entered'
  local input = io.open(entered)
  if input then
    input:close()
    return
  end
  assert(io.open(entered, 'w')):close()
  for _ = 1, 300 do
    input = io.open(MESSAGE_WORK_DIR .. '/work-release')
    if input then
      input:close()
      assert(io.open(MESSAGE_WORK_DIR .. '/work-completed', 'w')):close()
      return
    end
    kumo.time.sleep(0.05)
  end
  error 'test did not release message work'
end

kumo.on('log_disposition_message_work', function(msg)
  if MESSAGE_WORK == 'reinsert' and msg:recipient().user == 'queued' then
    -- Delayed logging happens after the message enters the scheduled queue.
    hold_message_work()
  elseif MESSAGE_WORK == 'defer' and msg:recipient().user == 'second' then
    -- Transient-failure logging happens before the message is requeued.
    hold_message_work()
  end
end)

kumo.on('throttle_insert_ready_queue', function(msg)
  if
    (MESSAGE_WORK == 'reinsert' or MESSAGE_WORK == 'reinsert-error')
    and msg:recipient().user == 'queued'
  then
    if msg:get_meta 'already_promoted' then
      if MESSAGE_WORK == 'reinsert-error' then
        hold_message_work()
        error 'test reinsertion failure'
      end
      msg:set_due(os.date('!%Y-%m-%dT%H:%M:%SZ', os.time() + 3600))
    else
      msg:set_meta('already_promoted', true)
    end
  end
end)

local function configure_dns(changes)
  local zones = {}
  for _, domain in ipairs {
    'none.example',
    'enforce.example',
    'testing.example',
    'secure.example',
    'secure2.example',
    'moving.example',
    'sibling.example',
  } do
    local zone = string.format(
      [[
$ORIGIN %s.
@ 600 MX 10 mx.shared.example.
]],
      domain
    )
    if os.getenv 'KUMOD_SHARED_MX_TWO_HOSTS' then
      zone = zone .. [[
@ 600 MX 20 other.shared.example.
]]
    end
    zones[#zones + 1] = {
      zone = zone,
      secure = domain == 'secure.example' or domain == 'secure2.example',
    }
  end
  local zone = [[
$ORIGIN shared.example.
mx 600 A 127.0.0.1
other 600 A 127.0.0.1
]]
  local tlsa = changes.tlsa or os.getenv 'KUMOD_SHARED_MX_TLSA'
  if tlsa then
    -- TLSA records are alternatives; the first need not match the peer.
    zone = zone
      .. string.format(
        [[
_%d._tcp.mx 600 TLSA 3 0 1 %s
_%d._tcp.mx 600 TLSA %s
]],
        SINK_PORT,
        string.rep('ff', 32),
        SINK_PORT,
        tlsa
      )
  end
  zones[#zones + 1] = { zone = zone, secure = true }
  local config = { zones = zones }
  if changes.servfail then
    config.servfail =
      { string.format('_%d._tcp.mx.shared.example', SINK_PORT) }
  end
  kumo.dns.configure_test_resolver(config)
end

local function configure_sts(changes)
  local policies = {
    -- TestResolver gives MX answers a fixed expiry. A short policy max_age
    -- lets the queued-message test expire the combined MX/policy snapshot.
    ['moving.example'] = string.format(
      [[
version: STSv1
mode: none
max_age: %s
]],
      POLICY_MAX_AGE
    ),
    ['enforce.example'] = [[
version: STSv1
mode: enforce
mx: mx.shared.example
max_age: 600
]],
    ['testing.example'] = [[
version: STSv1
mode: testing
mx: mx.shared.example
max_age: 600
]],
  }
  if changes.allowed_mx then
    policies['moving.example'] = string.format(
      [[
version: STSv1
mode: enforce
mx: %s
max_age: %s
]],
      changes.allowed_mx,
      POLICY_MAX_AGE
    )
  end
  kumo.dns.configure_test_mta_sts(policies)
end

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }
  kumo.start_esmtp_listener {
    listen = '127.0.0.1:0',
    relay_hosts = { '0.0.0.0/0' },
  }
  kumo.start_http_listener { listen = '127.0.0.1:0' }
  kumo.configure_local_logs {
    log_dir = TEST_DIR .. '/logs',
    max_segment_duration = '1s',
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
  if MESSAGE_WORK then
    kumo.configure_log_disposition_hook {
      name = 'message_work',
      per_record = {
        Any = { enable = false },
        [MESSAGE_WORK == 'reinsert' and 'Delayed' or 'TransientFailure'] = {
          enable = true,
        },
      },
    }
  end
  configure_dns {}
  configure_sts {}
end)

kumo.on('smtp_server_message_received', function(msg)
  local routing_domain = os.getenv 'KUMOD_SHARED_MX_ROUTING_DOMAIN'
  if routing_domain and msg:recipient().user == 'second' then
    msg:set_meta('routing_domain', routing_domain)
  end
  -- Tests can change DNS between messages without restarting the active session.
  local control = os.getenv 'KUMOD_SHARED_MX_CONTROL'
  if control then
    local input = io.open(control)
    if input then
      local changes = kumo.json_parse(input:read '*a')
      input:close()
      configure_dns(changes)
      configure_sts(changes)
    end
  end
end)

kumo.on('get_queue_config', function()
  return kumo.make_queue_config {
    retry_interval = '1h',
    -- Do not let a deduplicating queue hide repeated insertion in these tests.
    strategy = MESSAGE_WORK and 'TimerWheel' or nil,
  }
end)

kumo.on('get_egress_path_config', function()
  return kumo.make_egress_path {
    smtp_port = SINK_PORT,
    prohibited_hosts = {},
    enable_tls = os.getenv 'KUMOD_SHARED_MX_TLS' or 'Opportunistic',
    enable_dane = true,
    enable_mta_sts = os.getenv 'KUMOD_SHARED_MX_NO_STS' ~= 'true',
    tls_prefer_openssl = os.getenv 'KUMOD_SHARED_MX_OPENSSL' == 'true',
    remember_broken_tls = os.getenv 'KUMOD_SHARED_MX_REMEMBER_BROKEN_TLS',
    opportunistic_tls_reconnect_on_failed_handshake = os.getenv 'KUMOD_SHARED_MX_TLS_RECONNECT'
      == 'true',
    connection_limit = 1,
    aggressive_connection_opening = false,
    idle_timeout = '30s',
  }
end)
