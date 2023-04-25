# An Example Configuration

A default policy file is not published with KumoMTA to prevent the average installation from having an excess of commented-out boilerplate from filling production configurations.

The following serves as an example of a complete base policy for a functional installation that addresses common use cases for a typical installation. This is not intended as a copy/paste policy file, but as an example to direct new users in developing their server policy file.

The content of this example will be detailed in the following sections of this chapter, links will be in the comments of the example policy.

## The Example Server Policy

```lua
-- NOTE: This example policy is not meant to be used as-is, and will require some editing.
-- We strongly recommend reading the User Guide chapter on configuration before working with
-- this example policy. See https://docs.kumomta.com/userguide/configuration

-- This require statement is needed in any script passed to KumoMTA.
-- Includes from this policy script will not need this declared again.
local kumo = require 'kumo'

-- CALLED ON STARTUP, ALL ENTRIES WITHIN init REQUIRE A SERVER RESTART WHEN CHANGED.
kumo.on('init', function()
  -- Define the default "data" spool location; this is where
  -- message bodies will be stored.
  -- See https://docs.kumomta.com/userguide/configuration/spool/

  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumomta/data',
    kind = 'RocksDB',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored.

  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumomta/meta',
    kind = 'RocksDB',
  }

  -- Configure logging to local disk. Separating spool and logs to separate
  -- disks helps reduce IO load and can help performance.
  -- See https://docs.kumomta.com/userguide/configuration/logging/

  kumo.configure_local_logs {
    log_dir = '/var/log/kumomta',
    -- headers = { 'Subject', 'X-Customer-ID' },
  }

  -- Configure bounce classification.
  -- See https://docs.kumomta.com/userguide/configuration/bounce/

  kumo.configure_bounce_classifier {
    files = {
      '/opt/kumomta/share/bounce_classifier/iana.toml',
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
    hostname = 'mail.example.com',

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

  -- Add an IPv6 Listener
  kumo.start_esmtp_listener {
    listen = '[::]:25',
    relay_hosts = { '::1' },
  }

  -- Configure the sending IP addresses that will be used by KumoMTA to
  -- connect to remote systems. Note that defining sources and pools does
  -- nothing without some form of policy in effect to assign messages to
  -- the source pools you have defined.
  -- See https://docs.kumomta.com/userguide/configuration/sendingips/

  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
    ehlo_domain = 'mta1.examplecorp.com',
  }

  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '10.0.0.2',
    ehlo_domain = 'mta2.examplecorp.com',
  }

  -- IPv6 is also supported.
  kumo.define_egress_source {
    name = 'ip-3',
    source_address = '2001:db8:3333:4444:5555:6666:7777:8888',
    ehlo_domain = 'mta3.examplecorp.com',
  }

  kumo.define_egress_pool {
    name = 'TenantOne',
    entries = {
      { name = 'ip-2' },
      { name = 'ip-3' },
    },
  }

  kumo.define_egress_pool {
    name = 'TenantTwo',
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

-- Configure queue management settings. These are not throttles, but instead
-- how messages flow through the queues. This example assigns pool based
-- on tenant name, and customized message expiry for a specific tenant.
-- See https://docs.kumomta.com/userguide/configuration/queuemanagement/

local TENANT_PARAMS = {
  TenantOne = {
    max_age = '5 minutes',
  },
}

kumo.on('get_queue_config', function(domain, tenant, campaign)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
    egress_pool = tenant,
  }
  merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)

-- Configure traffic shaping.
-- WARNING: THIS WILL NOT LOAD WITHOUT AN ADDITIONAL SCRIPT IN PLACE
-- SEE https://docs.kumomta.com/userguide/configuration/trafficshaping/

local shaping = require 'shaping'
kumo.on('get_egress_path_config', shaping:setup_json())

-- Configure DKIM signing. In this case we use a simple approach of a path
-- defined by tokens, with each domain configured in the definition. This is
-- executed whether the message arrived by SMTP or API.
-- See https://docs.kumomta.com/userguide/configuration/dkim/

-- Edit this table to add more signing domains and their selector.
local DKIM_CONFIG = {
  ['examplecorp.com'] = 'dkim1024',
  ['kumocorp.com'] = 's1024',
}

function dkim_sign(msg)
  local sender_domain = msg:from_header().domain
  local selector = DKIM_CONFIG[sender_domain]

  if not selector then
    return false -- DON'T SIGN WITHOUT A VALID SELECTOR
  end

  local signer = kumo.dkim.rsa_sha256_signer {
    domain = sender_domain,
    selector = selector,
    headers = { 'From', 'To', 'Subject' },
    key = string.format(
      '/opt/kumomta/etc/dkim/%s/%s.key',
      sender_domain,
      selector
    ),
  }
  msg:dkim_sign(signer)
end

kumo.on('smtp_server_message_received', function(msg)
  -- Assign tenant based on X-Tenant header.
  local tenant = msg:get_first_named_header_value 'X-Tenant'
  if tenant then
    msg:set_meta('tenant', tenant)
  end

  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_sign(msg)
end)

kumo.on('http_message_generated', function(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_sign(msg)
end)
```
