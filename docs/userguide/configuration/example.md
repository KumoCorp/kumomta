# An Example Configuration

A default policy file is not published with KumoMTA to prevent the average installation from having an excess of commented-out boilerplate from filling production configurations.

The following serves as an example of a complete base policy for a functional installation that addresses common use cases for a typical installation. This is not intended as a copy/paste policy file, but as an example to direct new users in developing their server policy file.

The content of this example will be detailed in the following sections of this chapter, links will be in the comments of the example policy.

## The Example Server Policy

```lua
-- This require statement is needed in any script passed to KumoMTA.
-- Includes from this policy script will not need this declared again.
local kumo = require 'kumo'

-- CALLED ON STARTUP, ALL ENTRIES WITHIN init REQUIRE A REFRESH WHEN CHANGED.
kumo.on('init', function()
  -- Define the default "data" spool location; this is where
  -- message bodies will be stored.
  -- See https://docs.kumomta.com/userguide/configuration/spool/

  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
    flush = false,
    kind = 'RocksDB',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored.

  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
    flush = false,
    kind = 'RocksDB',
  }

  -- Configure logging to local disk. Separating spool and logs to separate
  -- disks helps reduce IO load and can help performance.
  -- See https://docs.kumomta.com/userguide/configuration/logging/

  kumo.configure_local_logs {
    log_dir = '/var/tmp/kumo-logs',
    -- headers = { 'Subject', 'X-Customer-ID' },
  }

  -- Configure bounce classification.
  -- See https://docs.kumomta.com/userguide/configuration/bounce/

  kumo.configure_bounce_classifier {
    files = {
      '/etc/kumo/bounce_rules.toml',
    },
  }

  -- Configure HTTP Listeners for injection and management APIs.
  -- See https://docs.kumomta.com/userguide/configuration/httplisteners/

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }
  kumo.start_http_listener {
    use_tls = true,
    listen = '0.0.0.0:8001',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }

  -- Define an SMTP listener. Can be used multiple times with different
  -- parameters to define multiple listeners!
  -- See https://docs.kumomta.com/userguide/configuration/smtplisteners/

  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',

    -- override the default set of relay hosts
    relay_hosts = { '127.0.0.1', '192.168.1.0/24' },

    -- Configure the domains that are allowed for outbound & inbound relay,
    -- Out-Of-Band bounces, and Feedback Loop Reports.
    -- See https://docs.kumomta.com/userguide/configuration/domains/
    domains = {
      ['examplecorp.com'] = {
        -- allow relaying mail from any source, so long as it is
        -- addressed to examplecorp.com, for inbound mail.
        relay_to = true,
      },
      ['send.examplecorp.com'] = {
        -- relay to anywhere, so long as the sender domain is
        -- send.examplecorp.com and the connected peer matches one of the
        -- listed CIDR blocks, helps prevent abuse by less trusted peers.
        relay_from = { '10.0.0.0/24' },
      },
      ['bounce.examplecorp.com'] = {
        -- accept and log OOB bounce reports sent to bounce.examplecorp.com
        log_oob = true,
      },
      ['fbl.examplecorp.com'] = {
        -- accept and log ARF feedback reports sent to fbl.examplecorp.com
        log_arf = true,
      },
    },
  }

  -- Configure the sending IP addresses that will be used by KumoMTA to
  -- connect to remote systems. Note that defining sources and pools does
  -- nothing without some form of policy in effect to assign messages to
  -- the source pools you have defined.
  -- See https://docs.kumomta.com/userguide/configuration/sendingips/

  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
  }

  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '10.0.0.2',
  }

  kumo.define_egress_source {
    name = 'ip-3',
    source_address = '10.0.0.3',
  }

  kumo.define_egress_source {
    name = 'ip-4',
    source_address = '10.0.0.4',
  }

  kumo.define_egress_source {
    name = 'ip-5',
    source_address = '10.0.0.5',
  }

  kumo.define_egress_pool {
    name = 'MyPool',
    entries = {
      { name = 'ip-1' },
      { name = 'ip-2' },
      { name = 'ip-3' },
      { name = 'ip-4' },
      { name = 'ip-5' },
    },
  }

  kumo.define_egress_pool {
    name = 'MySubPool',
    entries = {
      { name = 'ip-1' },
      { name = 'ip-2' },
    },
  }

  -- Use shared throttles rather than in-process throttles, do not enable
  -- without first installing and configuring redis.
  -- See https://docs.kumomta.com/reference/kumo/configure_redis_throttles/
  -- kumo.configure_redis_throttles { node = 'redis://127.0.0.1/' }

end) -- END OF THE INIT EVENT

-- Configure traffic shaping, typically on a global basis for each
-- destination domain. Throttles will apply on a per-ip basis.
-- See https://docs.kumomta.com/userguide/configuration/trafficshaping/

-- INSERT THE TRAFFIC SHAPING EXAMPLE HERE

-- Not the final form of this API, but this is currently how
-- we retrieve configuration used when making outbound
-- connections
kumo.on('get_egress_path_config', function(domain, site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
    -- max_message_rate = '5/min',
    idle_timeout = '5s',
    max_connections = 1024,
    -- max_deliveries_per_connection = 5,

    -- hosts that we should consider to be poison because
    -- they are a mail loop.
    prohibited_hosts = { "127.0.0.0/8", "::1" },
  }
end)

-- Configure queue management settings. These are not throttles, but instead
-- how messages flow through the queues.
-- See https://docs.kumomta.com/userguide/configuration/queuemanagement/

-- Not the final form of this API, but this is currently how
-- we retrieve configuration used for managing a queue.
kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {
    -- Age out messages after being in the queue for 2 minutes
    max_age = "2 minutes",
    retry_interval = "2 seconds",
    max_retry_interval = "8 seconds",
    egress_pool = 'MyPool',
  }
end)

-- Configure DKIM signing. In this case we use a simple approach of a path
-- defined by tokens, with each domain configured in the definition. This is
-- executed whether the message arrived by SMTP or API.
-- See https://docs.kumomta.com/userguide/configuration/dkim/

function dkim_sign(msg)
  -- Edit this table to add more signing domains and their selector.
  local DKIM_CONFIG = {
    ["examplecorp.com"] = "dkim1024",
    ["kumocorp.com"] = "s1024",
  }

  local sender_domain = msg:sender().domain
  local selector = DKIM_CONFIG[sender_domain] or 'default'

  if selector == 'default' then
    return false  -- DON'T SIGN WITHOUT A SELECTOR
  end

  local signer = kumo.dkim.rsa_sha256_signer {
    domain = sender_domain,
    selector = selector,
    headers = { 'From', 'To', 'Subject' },
    key = string.format('/opt/kumomta/etc/dkim/%s/%s.key', sender_domain, selector),
  }
  msg:dkim_sign(signer)
end

kumo.on('smtp_server_message_received', function(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_sign(msg)
end)

kumo.on('http_message_generated', function(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_sign(msg)
end)
```
