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
    kind = 'RocksDB',
  }

  -- Define the default "meta" spool location; this is where
  -- message envelope and metadata will be stored.

  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
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
      '/opt/kumomta/etc/bounce_rules.toml',
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
    ehlo_domain = 'mta1.examplecorp.com',
  }

  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '10.0.0.2',
    ehlo_domain = 'mta3.examplecorp.com',
  }

  kumo.define_egress_source {
    name = 'ip-3',
    source_address = '10.0.0.3',
    ehlo_domain = 'mta1.examplecorp.com',
  }

  kumo.define_egress_source {
    name = 'ip-4',
    source_address = '10.0.0.4',
    ehlo_domain = 'mta4.examplecorp.com',
  }

  kumo.define_egress_source {
    name = 'ip-5',
    source_address = '10.0.0.5',
    ehlo_domain = 'mta5.examplecorp.com',
  }

  kumo.define_egress_pool {
    name = 'TenantOne',
    entries = {
      { name = 'ip-3' },
      { name = 'ip-4' },
      { name = 'ip-5' },
    },
  }

  kumo.define_egress_pool {
    name = 'TenantTwo',
    entries = {
      { name = 'ip-1' },
      { name = 'ip-2' },
      { name = 'ip-3' },
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

-- Throttles are part of an egress path, which is a combination of source
-- IP (egress_source) and destination. While an individual message will have
-- a named destination domain, queues are defined by using a site_name, which
-- is a pattern string that represents all MXes for the destination domain,
-- such as (alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com
-- This means that we configure throttles at the site_name level, which we
-- can map from the recipient domain name. This example stores throttle
-- configuration in a pair of lua tables that contain either the destination
-- domain, or the site name, depending on whether we expect the domain to
-- have a single domain name for all MX records, or to roll up many domains.

-- For each domain in the table, render the site name for lookup.
-- This will populate the MX_ROLLUP table with the correct queue identifiers
-- for all domains that are hosted by that top level domain. Note we're just
-- listing one top-level domain, not every possible MX pattern.

-- Once lookup is done, the table will look similar to this:
-- local MX_ROLLUP = {
--   ["gmail.com"] = "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com",
--   ["yahoo.com"] = "(mta5|mta6|mta7).am0.yahoodns.net"
-- }

local MX_ROLLUP = {}
for _, domain in ipairs { 'gmail.com', 'yahoo.com', 'outlook.com' } do
  MX_ROLLUP[domain] = kumo.dns.lookup_mx(domain).site_name
end

-- Global domain default traffic shaping rules.
-- This table is keyed by either site name or domain name.
-- Site names are looked up first, then domain names.
local SHAPE_BY_DOMAIN = {
  [MX_ROLLUP['gmail.com']] = {
    connection_limit = 3,
    max_deliveries_per_connection = 50,
  },
  [MX_ROLLUP['outlook.com']] = {
    connection_limit = 10,
  },
  [MX_ROLLUP['yahoo.com']] = {
    connection_limit = 10,
    max_deliveries_per_connection = 20,
  },
  ['comcast.net'] = {
    connection_limit = 2,
    max_deliveries_per_connection = 100,
    max_message_rate = '1/second',
  },
}

-- Per IP/Domain traffic shaping rules.
-- This table is keyed by the tuple of (site_name, source) or (domain, source).
-- Site names are looked up first, then domain names.
-- Values override/overlay those in SHAPE_BY_DOMAIN.
local SHAPE_BY_SOURCE = {
  [{ MX_ROLLUP['gmail.com'], 'ip-1' }] = {
    max_message_rate = '1000/hr',
  },
  [{ 'comcast.net', 'ip-2' }] = {
    max_message_rate = '10/second',
  },
}

function merge_into(src, dest)
  for k, v in pairs(src) do
    dest[k] = v
  end
end

kumo.on('get_egress_path_config', function(domain, egress_source, site_name)
  -- resolve parameters first based on the site, if any,
  -- then based on the domain, if any,
  -- otherwise use the system defaults
  local domain_params = SHAPE_BY_DOMAIN[site_name]
    or SHAPE_BY_DOMAIN[domain]
    or {}
  local source_params = SHAPE_BY_SOURCE[{ site_name, egress_source }]
    or SHAPE_BY_SOURCE[{ domain, egress_source }]
    or {}
  -- compose the source params over the domain params
  local params = {}
  merge_into(domain_params, params)
  merge_into(source_params, params)
  return kumo.make_egress_path(params)
end)

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
    egress_pool = tenant,
  }
  merge_into(TENANT_PARAMS[tenant], params)
  return kumo.make_queue_config(params)
end)

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
