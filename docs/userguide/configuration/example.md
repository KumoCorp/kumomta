# An Example Configuration

A default policy file is not published with KumoMTA to prevent the average
installation from having an excess of commented-out boilerplate from filling
production configurations.

The following serves as an example of a complete base policy for a functional
installation that addresses common use cases for a typical installation. This
is not intended as a copy/paste policy file, but as an example to direct new
users in developing their server policy file.

The content of this example will be detailed in the following sections of this
chapter, links will be in the comments of the example policy.

## The Example Server Policy

```lua
-- NOTE: This example policy is not meant to be used as-is, and will require some editing.
-- We strongly recommend reading the User Guide chapter on configuration before working with
-- this example policy. See https://docs.kumomta.com/userguide/configuration

-- This file must be written to /opt/kumomta/etc/policy/init.lua for use.

-- This require statement is needed in any script passed to KumoMTA.
-- Includes from this policy script will not need this declared again.
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- Load the policy helpers to simplify common configuration use cases
local shaping = require 'policy-extras.shaping'
local queue_module = require 'policy-extras.queue'
local listener_domains = require 'policy-extras.listener_domains'
local sources = require 'policy-extras.sources'
local dkim_sign = require 'policy-extras.dkim_sign'
local log_hooks = require 'policy-extras.log_hooks'

-- Load TSA shaper tools
local shaping_config = '/opt/kumomta/etc/policy/shaping.toml'
local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = { shaping_config },
}

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

  -- Configure publishing of TSA logs to automation daemon
  shaper.setup_publish()

  -- Configure logging to local disk. Separating spool and logs to separate
  -- disks helps reduce IO load and can help performance.
  -- See https://docs.kumomta.com/userguide/configuration/logging/

  kumo.configure_local_logs {
    log_dir = '/var/log/kumomta',
    max_segment_duration = '1 minute',
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
  }

  -- Add an IPv6 Listener
  kumo.start_esmtp_listener {
    listen = '[::]:25',
    relay_hosts = { '::1' },
  }

  -- Use shared throttles rather than in-process throttles, do not enable
  -- without first installing and configuring redis.
  -- See https://docs.kumomta.com/reference/kumo/configure_redis_throttles/
  -- kumo.configure_redis_throttles { node = 'redis://127.0.0.1/' }
end) -- END OF THE INIT EVENT

-- Send a JSON webhook to a local network host.
-- See https://docs.kumomta.com/userguide/operation/webhooks/
log_hooks:new_json {
  name = 'webhook',
  url = 'http://10.0.0.1:4242/log',
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
}

-- Configure listener domains for relay, oob bounces, and FBLs using the
-- listener_domains.lua policy helper.
-- WARNING: THIS WILL NOT LOAD WITHOUT THE listener_domains.toml FILE IN PLACE
-- SEE https://docs.kumomta.com/userguide/configuration/smtplisteners/
kumo.on(
  'get_listener_domain',
  listener_domains:setup { '/opt/kumomta/etc/listener_domains.toml' }
)

-- Configure traffic shaping using the shaping.lua policy helper.
-- WARNING: THIS WILL NOT LOAD WITHOUT AN ADDITIONAL SCRIPT IN PLACE
-- SEE https://docs.kumomta.com/userguide/configuration/trafficshaping/
local get_shaping_config = shaping:setup()

-- Configure the sending IP addresses that will be used by KumoMTA to
-- connect to remote systems using the sources.lua policy helper.
-- Note that defining sources and pools does nothing without some form of
-- policy in effect to assign messages to the source pools you have defined.
-- WARNING: THIS WILL NOT LOAD WITHOUT THE source.toml FILE IN PLACE
-- SEE https://docs.kumomta.com/userguide/configuration/sendingips/
sources:setup { '/opt/kumomta/etc/sources.toml' }

-- Configure queue management settings. These are not throttles, but instead
-- control how messages flow through the queues.
-- WARNING: THIS WILL NOT LOAD WITHOUT the queues.toml FILE IN PLACE
-- See https://docs.kumomta.com/userguide/configuration/queuemanagement/
local queue_helper =
  queue_module:setup { '/opt/kumomta/etc/policy/queues.toml' }

-- Configure DKIM signing. In this case we use the dkim_sign.lua policy helper.
-- WARNING: THIS WILL NOT LOAD WITHOUT the dkim_data.toml FILE IN PLACE
-- See https://docs.kumomta.com/userguide/configuration/dkim/
local dkim_signer = dkim_sign:setup { '/opt/kumomta/etc/dkim_data.toml' }

kumo.on('smtp_server_message_received', function(msg)
  queue_helper:apply(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)

kumo.on('http_message_generated', function(msg)
  queue_helper:apply(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)
```
