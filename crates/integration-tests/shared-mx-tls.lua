-- Shared ready queue with independently controlled recipient-domain policies.
local kumo = require 'kumo'
local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')
local DANE_SITE_NAMES = os.getenv 'KUMOD_SHARED_MX_DANE_SITE_NAMES' == 'true'

kumo.on('pre_init', function()
  kumo.dns.set_dane_enabled(DANE_SITE_NAMES)
end)

local function configure_dns(changes)
  local zones = {}
  for _, domain in ipairs {
    'none.example',
    'enforce.example',
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
    -- Site-name isolation probes port 25; delivery uses the ephemeral sink port.
    local ports = DANE_SITE_NAMES and { SINK_PORT, 25 } or { SINK_PORT }
    for _, port in ipairs(ports) do
      -- TLSA records are alternatives; the first need not match the peer.
      zone = zone
        .. string.format(
          [[
_%d._tcp.mx 600 TLSA 3 0 1 %s
_%d._tcp.mx 600 TLSA %s
]],
          port,
          string.rep('ff', 32),
          port,
          tlsa
        )
    end
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
    -- Keep the MX cache lifetime short for the promotion tests.
    ['moving.example'] = [[
version: STSv1
mode: none
max_age: 1
]],
    ['enforce.example'] = [[
version: STSv1
mode: enforce
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
max_age: 1
]],
      changes.allowed_mx
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
  configure_dns {}
  configure_sts {}
end)

kumo.on('smtp_server_message_received', function(msg)
  local routing_domain = os.getenv 'KUMOD_SHARED_MX_ROUTING_DOMAIN'
  if routing_domain and msg:recipient().user == 'second' then
    msg:set_meta('routing_domain', routing_domain)
  end
  if os.getenv 'KUMOD_SHARED_MX_PORT_QUALIFIED_ROUTING' == 'true' then
    msg:set_meta(
      'routing_domain',
      string.format('%s:%d', msg:recipient().domain, SINK_PORT)
    )
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
    retry_interval = os.getenv 'KUMOD_SHARED_MX_RETRY_INTERVAL' or '1h',
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
    connection_limit = 1,
    idle_timeout = '30s',
  }
end)
