-- A minimal source policy for the DANE integration tests.
--
-- It routes mail for `dane.example` via the real MX resolution path (so that
-- DANE engages; mx_list would bypass it) and installs a TestResolver whose
-- zone and DNSSEC secure status are driven by env vars set by the test:
--
--   KUMOD_DANE_TLSA     the TLSA rdata to publish (e.g. "3 1 1 <hex>"), or unset
--   KUMOD_DANE_SECURE   "true" to mark the zone DNSSEC validated
--   KUMOD_DANE_SERVFAIL "true" to make the TLSA lookup return SERVFAIL
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local SINK_PORT = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT')

local function configure_resolver()
  -- When set, the MX host is a CNAME into a separate, unsigned zone (its A
  -- record is therefore insecure), while the MX and TLSA records remain in the
  -- secure dane.example zone. This models RFC 7672 section 2.2.2: DANE must
  -- still engage at the original MX name via the secure CNAME alias.
  local cname_unsigned = os.getenv 'KUMOD_DANE_CNAME_UNSIGNED' == 'true'

  local mx_address_record = 'mx 3600 IN A 127.0.0.1\n'
  if cname_unsigned then
    mx_address_record = 'mx 3600 IN CNAME target.unsigned.example.\n'
  end

  -- The TLSA record is published at _<port>._tcp.<mxhost>, where <port> is the
  -- port we actually connect on; we set the egress source remote_port to the
  -- sink port, so use that here too.
  local zone = string.format(
    [[
$ORIGIN dane.example.
@ 3600 IN MX 10 mx.dane.example.
%s]],
    mx_address_record
  )

  local tlsa = os.getenv 'KUMOD_DANE_TLSA'
  if tlsa then
    zone = zone
      .. string.format('_%d._tcp.mx 3600 IN TLSA %s\n', SINK_PORT, tlsa)
  end

  local zones = {
    {
      zone = zone,
      secure = os.getenv 'KUMOD_DANE_SECURE' == 'true',
    },
  }

  if cname_unsigned then
    zones[#zones + 1] = {
      zone = [[
$ORIGIN unsigned.example.
target 3600 IN A 127.0.0.1
]],
      secure = false,
    }
  end

  local config = {
    zones = zones,
  }

  if os.getenv 'KUMOD_DANE_SERVFAIL' == 'true' then
    config.servfail = { string.format('_%d._tcp.mx.dane.example', SINK_PORT) }
  end

  kumo.dns.configure_test_resolver(config)
end

kumo.on('init', function()
  -- Keep accounting state isolated to this test; the defaults live in a shared,
  -- possibly non-writable location.
  kumo.configure_accounting_db_path ':memory:'
  kumo.aaa.configure_acct_log {
    log_dir = TEST_DIR .. '/acct',
    max_segment_duration = '1s',
  }

  configure_resolver()

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
end)

kumo.on('get_queue_config', function(domain, _tenant, _campaign)
  -- An explicit egress pool ensures our get_egress_source (and thus
  -- remote_port) is honored.
  local params = {
    egress_pool = 'dane',
  }
  if domain == 'mxlist.example' then
    -- Route via a locally-configured mx_list host instead of DNS MX. DANE
    -- applies because we explicitly assert the selection is trusted and the
    -- host's A/AAAA + TLSA records are securely resolved.
    params.protocol = {
      smtp = {
        mx_list = { 'mx.dane.example:' .. SINK_PORT },
        treat_mx_list_as_secure = true,
      },
    }
  elseif domain == 'mxlistinsecure.example' then
    params.protocol = {
      smtp = {
        mx_list = { 'mx.dane.example:' .. SINK_PORT },
        treat_mx_list_as_secure = false,
      },
    }
  end
  -- Otherwise (dane.example) use the default smtp protocol, which resolves the
  -- routing domain's MX records via DNS (our TestResolver).
  return kumo.make_queue_config(params)
end)

kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = { { name = 'dane' } },
  }
end)

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    -- Direct the connection at the sink's ephemeral port. This is also the
    -- port used to form the TLSA query name.
    remote_port = SINK_PORT,
  }
end)

kumo.on('get_egress_path_config', function(_domain, _source_name, _site_name)
  return kumo.make_egress_path {
    enable_dane = true,
    -- When DANE does not apply (insecure chain), fall back to opportunistic
    -- TLS that tolerates the sink's self-signed certificate.
    enable_tls = 'OpportunisticInsecure',
    -- Allow connecting to the loopback sink.
    prohibited_hosts = {},
  }
end)
