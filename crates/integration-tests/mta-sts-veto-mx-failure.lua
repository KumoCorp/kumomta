-- Hold a plaintext dispatcher after its creator's RCPT rejection, admit a
-- sibling to the shared ready queue, then fail that sibling's MX lookup.
local kumo = require 'kumo'
local TEST_DIR = os.getenv 'KUMOD_TEST_DIR'
local MODE = os.getenv 'KUMOD_TEST_STS_MODE'
local MAX_AGE = tonumber(os.getenv 'KUMOD_TEST_STS_MAX_AGE') or 86400
local TARGET = 'target.example.com'

local function write_marker(name, content)
  local path = TEST_DIR .. '/' .. name
  local file = assert(io.open(path .. '.tmp', 'w'))
  file:write(content)
  file:close()
  assert(os.rename(path .. '.tmp', path))
end

local function released()
  local file = io.open(TEST_DIR .. '/release-dispatcher', 'r')
  if file then
    file:close()
    return true
  end
  return false
end

kumo.on('init', function()
  kumo.configure_accounting_db_path(TEST_DIR .. '/accounting.db')
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
  kumo.dns.configure_test_resolver {
    [[
$ORIGIN example.com.
nopolicy 600 MX 10 shared.example.com.
target     1 MX 10 shared.example.com.
shared   600 A 127.0.0.1
]],
  }
  local policies = {}
  if MODE ~= 'absent' then
    -- The hook simulates fetching an authenticated policy body and populates
    -- the real policy cache. It does not exercise TXT/HTTPS authentication.
    policies[TARGET] = string.format(
      [[
version: STSv1
mode: %s
mx: shared.example.com
max_age: %d
]],
      MODE,
      MAX_AGE
    )
  end
  kumo.dns.configure_test_mta_sts(policies)
end)

kumo.on('get_queue_config', function(_domain)
  return kumo.make_queue_config { retry_interval = '1h' }
end)

kumo.on('smtp_server_message_received', function(msg)
  if msg:recipient().domain == TARGET then
    -- Prime the MX/policy cache used by promotion. An existing shared queue
    -- does not call get_egress_path_config again for the joining domain.
    local mx = kumo.dns.lookup_mx(TARGET)
    write_marker('promotion-mx.json', kumo.serde.json_encode(mx))
  end
end)

kumo.on('get_egress_path_config', function(_domain)
  return kumo.make_egress_path {
    enable_tls = 'Opportunistic',
    enable_mta_sts = true,
    prohibited_hosts = {},
    smtp_port = tonumber(os.getenv 'KUMOD_SMTP_SINK_PORT'),
    connection_limit = 1,
  }
end)

kumo.on('smtp_client_rewrite_delivery_status', function(response, domain)
  if domain ~= 'nopolicy.example.com' then
    return
  end
  assert(response:find('450', 1, true), response)
  write_marker('dispatcher-held', response)
  -- This is a bounded handshake, not a mandatory sleep. The test releases the
  -- dispatcher only after the target message has reached the shared ready queue.
  for _ = 1, 600 do
    if released() then
      break
    end
    kumo.sleep(0.05)
  end
  assert(released(), 'test did not release dispatcher')

  kumo.dns.configure_test_resolver {
    zones = {
      [[
$ORIGIN shared.example.com.
@ 600 A 127.0.0.1
]],
    },
    servfail = { TARGET },
  }
  for _ = 1, 200 do
    local ok, err = pcall(kumo.dns.lookup_mx, TARGET)
    if not ok then
      assert(tostring(err):find('Server Failure', 1, true), tostring(err))
      write_marker('mx-failed', tostring(err))
      return
    end
    -- Wait for the short MX TTL, not the much longer valid policy lifetime.
    kumo.sleep(0.05)
  end
  error 'target MX result did not expire'
end)
