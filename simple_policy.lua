-- THIS IS NOT THE FILE YOU ARE LOOKING FOR!
-- This file is wez's local hacking/testing config.
-- You do not want to use this. It is not appropriate
-- for your production needs.
local kumo = require 'kumo'
package.path = 'assets/?.lua;' .. package.path
local shaping = require 'policy-extras.shaping'
local listener_domains = require 'policy-extras.listener_domains'

kumo.on('pre_init', function()
  kumo.set_httpinject_recipient_rate_limit 'local:6,000/s'
  kumo.set_httpinject_threads(math.ceil(kumo.available_parallelism() / 2))
  kumo.set_readyq_threads(math.ceil(kumo.available_parallelism() / 2))
end)

kumo.on(
  'get_listener_domain',
  listener_domains:setup {
    {
      ['auth-send.example.com'] = {
        relay_from_authz = { 'daniel' },
      },
    },
  }
)

local shaper = shaping:setup_with_automation {
  no_default_files = true,
  -- extra_files = { 'assets/policy-extras/shaping.toml' },
}

local sources = require 'policy-extras.sources'
sources:setup {
  {
    pool = {
      pool0 = {
        source1 = { weight = 10 },
        source2 = { weight = 20 },
        source3 = { weight = 30 },
      },
    },
    source = {
      source1 = {},
      source2 = {},
      source3 = {},
    },
  },
}

local queue_module = require 'policy-extras.queue'
local queue_helper = queue_module:setup {
  -- '/tmp/invalid/file.toml',
  {
    scheduling_header = 'X-Schedule',
    tenant = {
      mytenant = {
        egress_pool = 'pool0',
      },
    },
    queue = {
      default = {
        egress_pool = 'pool0',
        -- refresh_interval = '2 hours',
        strategy = 'SingletonTimerWheelV2',
        retry_interval = '5m',
        -- reap_interval = '10s',
      },
    },
  },
}

local dkim_sign = require 'policy-extras.dkim_sign'
local dkim_signer = dkim_sign:setup {
  base = {
    selector = 'woot',
    headers = { 'From', 'To', 'Subject' },
    additional_signatures = { 'MyEsp' },
  },
  domain = {
    ['example.com'] = {
      policy = 'Always',
      filename = 'example-private-dkim-key.pem',
      -- algo = 'sha256',
      -- policy = "SignOnlyIfInDNS",
    },
  },
  signature = {
    MyEsp = {
      domain = 'example.com',
      policy = 'OnlyIfMissingDomainBlock',
      filename = 'example-private-dkim-key.pem',
    },
  },
}

-- Called on startup to initialize the system
kumo.on('init', function()
  kumo.set_config_monitor_globs {
    '/home/wez/kumocorp/kumomta/**/*.{lua,toml}',
  }

  kumo.set_lua_gc_on_put(1)
  kumo.configure_accounting_db_path(os.tmpname())
  kumo.configure_bounce_classifier {
    files = {
      '/home/wez/kumocorp/kumomta/assets/community/bounces.toml',
      '/home/wez/kumocorp/kumomta/assets/bounce_classifier/iana.toml',
    },
  }

  -- Define a listener.
  -- Can be used multiple times with different parameters to
  -- define multiple listeners!
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2025',
    -- Override the hostname reported in the banner and other
    -- SMTP responses:
    -- hostname="mail.example.com",

    -- override the default set of relay hosts
    relay_hosts = { '127.0.0.1', '192.168.1.0/24' },

    -- Customize the banner.
    -- The configured hostname will be automatically
    -- prepended to this text.
    banner = 'Welcome to KumoMTA!',

    -- Unsafe! When set to true, don't save to spool
    -- at reception time.
    -- Saves IO but may cause you to lose messages
    -- if something happens to this server before
    -- the message is spooled.
    -- deferred_spool = true,

    -- max_recipients_per_message = 1024
    -- max_messages_per_connection = 10000,

    peer = {
      ['127.0.0.1'] = {
        banner = 'Welcome to loopback!',
        allow_xclient = true,
      },
      ['192.168.1.0/24'] = {
        banner = 'Welcome to lan!',
      },
    },
  }

  local do_logging = true
  if do_logging then
    kumo.configure_local_logs {
      log_dir = '/var/tmp/kumo-logs',
      max_segment_duration = '1s',
      back_pressure = 512 * 1024,
    }
  end

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1', '192.168.1.0/24', '10.0.0.0/8' },
  }
  kumo.start_http_listener {
    use_tls = true,
    listen = '0.0.0.0:8001',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }

  -- Define the default "data" spool location; this is where
  -- message bodies will be stored
  --
  -- 'flush' can be set to true to cause fdatasync to be
  -- triggered after each store to the spool.
  -- The increased durability comes at the cost of throughput.
  --
  -- kind can be 'LocalDisk' (currently the default) or 'RocksDB'.
  --
  -- LocalDisk stores one file per message in a filesystem hierarchy.
  -- RocksDB is a key-value datastore.
  --
  -- RocksDB has >4x the throughput of LocalDisk, and enabling
  -- flush has a marginal (<10%) impact in early testing.
  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
    flush = false,
    kind = 'RocksDB',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored
  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
    flush = false,
    kind = 'RocksDB',
  }

  -- Use shared throttles rather than in-process throttles
  -- kumo.configure_redis_throttles { node = 'redis://127.0.0.1/' }
end)

--[[

-- Called to validate the helo and/or ehlo domain
kumo.on('smtp_server_ehlo', function(domain)
  -- print('ehlo domain is', domain)
  -- Use kumo.reject to return an error to the EHLO command
  -- kumo.reject(420, 'wooooo!')
end)

-- Called to validate the sender
kumo.on('smtp_server_mail_from', function(sender)
  -- print('sender', tostring(sender))
  -- kumo.reject(420, 'wooooo!')
end)

-- Called to validate a recipient
kumo.on('smtp_server_rcpt_to', function(rcpt)
  -- print('rcpt', tostring(rcpt))
end)

]]

local function common_processing(msg)
  local from_header = msg:from_header()
  if not from_header then
    kumo.reject(
      552,
      '5.6.0 DKIM signing requires a From header, but it is missing from this message'
    )
  end

  -- local verify = msg:dkim_verify()
  -- print('dkim', kumo.json_encode_pretty(verify))
  -- msg:add_authentication_results(msg:get_meta 'hostname', verify)
  -- print(msg:get_first_named_header_value 'Authentication-Results')
  -- print(msg:get_data())

  --[[
  local failed = msg:check_fix_conformance(
    -- check for and reject messages with these issues:
    'MISSING_COLON_VALUE',
    -- fix messages with these issues:
    'LINE_TOO_LONG|NAME_ENDS_WITH_SPACE|NEEDS_TRANSFER_ENCODING|NON_CANONICAL_LINE_ENDINGS|MISSING_DATE_HEADER|MISSING_MESSAGE_ID_HEADER|MISSING_MIME_VERSION'
  )
  if failed then
    kumo.reject(552, string.format('5.6.0 %s', failed))
  end
  ]]

  -- print('id', msg:id(), 'sender', tostring(msg:sender()))
  -- print(msg:get_meta 'authn_id')
  -- msg:set_meta('routing_domain', 'outlook.com')

  -- Import scheduling information from X-Schedule and
  -- then remove that header from the message
  msg:import_scheduling_header('X-Schedule', true)

  -- msg:set_meta('tenant', 't' .. tostring(math.random(1000)))
  -- msg:set_meta('campaign', 'c' .. tostring(math.random(1000)))

  local do_signing = true
  if do_signing then
    local signer = kumo.dkim.rsa_sha256_signer {
      domain = msg:from_header().domain,
      selector = 'default',
      headers = { 'From', 'To', 'Subject' },
      -- Using a file:
      key = 'example-private-dkim-key.pem',
      -- Using HashiCorp Vault:
      --[[
      key = {
        vault_mount = "secret",
        vault_path = "dkim/" .. msg:sender().domain,
        -- Optional: specify a custom key name (defaults to "key")
        -- vault_key = "private_key"
      }
      ]]
    }
    msg:dkim_sign(signer)
  end

  -- msg:set_meta('queue', 'null')

  -- set/get metadata fields
  -- msg:set_meta('X-TestMSG', 'true')
  -- print('meta X-TestMSG is', msg:get_meta 'X-TestMSG')
end

-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  common_processing(msg)
end)

kumo.on('http_message_generated', function(msg)
  common_processing(msg)
end)

-- Not the final form of this API, but this is currently how
-- we retrieve configuration used when making outbound
-- connections
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    if routing_domain == 'generator.kumomta.internal' then
      return kumo.make_egress_path {
        connection_limit = kumo.available_parallelism(),
        refresh_strategy = 'Epoch',
        max_ready = 80000,
      }
    end

    local skip_make = true
    local params = shaper.get_egress_path_config(
      routing_domain,
      egress_source,
      site_name,
      skip_make
    )

    -- print('get_egress_path_config', routing_domain, egress_source, site_name)
    -- enable_tls = 'OpportunisticInsecure',
    params.enable_tls = 'Disabled'
    params.enable_mta_sts = false
    -- max_message_rate = '5/min',
    params.connection_limit = 300
    -- max_connection_rate = '1/s',
    params.max_ready = 80000
    -- smtp_port = 2026,
    -- max_deliveries_per_connection = 5,

    -- hosts that we should consider to be poison because
    -- they are a mail loop. The default for this is
    -- { "127.0.0.0/8", "::1" }, but it is emptied out
    -- in this config because we're using this to test
    -- with fake domains that explicitly return loopback
    -- addresses!
    params.prohibited_hosts = {}

    refresh_strategy = 'Epoch'
    return kumo.make_egress_path(params)
  end
)

-- A really simple inline auth "database" for very basic HTTP authentication
function simple_auth_check(user, password)
  local password_database = {
    ['daniel'] = 'tiger',
  }
  if password == '' then
    return false
  end
  return password_database[user] == password
end

-- Consult a hypothetical sqlite database that has an auth table
-- with user and pass fields
function sqlite_auth_check(user, password)
  local sqlite = require 'sqlite'
  local db = sqlite.open '/tmp/auth.db'
  local result = db:execute(
    'select user from auth where user=? and pass=?',
    user,
    password
  )
  return result[1] == user
end

-- Use this to lookup and confirm a user/password credential
-- used with the http endpoint
kumo.on('http_server_validate_auth_basic', function(user, password)
  return simple_auth_check(user, password)

  -- or use sqlite
  -- return sqlite_auth_check(user, password)
end)

-- Use this to lookup and confirm a user/password credential
-- when the client attempts SMTP AUTH PLAIN
kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return simple_auth_check(authc, password)
  -- or use sqlite
  -- return sqlite_auth_check(authc, password)
end)
