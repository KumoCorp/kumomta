# Lua Policy Helpers

KumoMTA is designed around the concept of configuration as code, where all configuration of KumoMTA is implemented through Lua policy rather than static text files.

Configuration as code offers numerous advantages, including late loading of config for lower memory consumption and minimal reloads and direct data source connectivity to make your KumoMTA instances a well-integrated part of your DevOps environment rather than a black box that requires automated config file updates and reload commands to be issued.

Configuration as code enables advanced use cases such as [storing your DKIM signing keys in HashiCorp Vault](../policy/hashicorp_vault.md) for realtime DKIM signing and checking SMTP Authentication credentials against a live data source.

While configuration as code provides extreme flexibility and deep integration capabilities, that can come at the cost of complexity. In order to make KumoMTA more accessible for those who are accustomed to a static configuration file and don't need deeper integration, we have developed a set of policy helpers. These helpers are premade Lua policy scripts that implement common use cases by reading formatted TOML and JSON files to configure KumoMTA.

## An Example

For example, DKIM signing can be implemented using Lua directly in the following example:

```lua
-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```

Or even dynamically configured against a data source:

```lua
function get_key(domain, selector)
  local db = sqlite:open '/opt/kumomta/etc/dkim/keys.db'
  local result = db:execute(
    'select data from keys where domain=? and selector=?',
    domain,
    selector
  )
  return result[1]
end

local sqlite_signer = kumo.dkim.rsa_sha256_signer {
  key = {
    key_data = get_key(msg:from_header().domain, 'default'),
  },
}
```

A more straightforward implementation can be performed by using the DKIM signing helper:

```lua
local dkim_sign = require 'policy-extras.dkim_sign'
local dkim_signer = dkim_sign:setup { '/opt/kumomta/etc/dkim_data.toml' }

kumo.on('smtp_server_message_received', function(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)

kumo.on('http_message_generated', function(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)
```

With the helper code in place, users can manage a simple TOML or JSON file to control DKIM signing:

```toml
[domain."example.com"]
selector = 'dkim1024'
headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]
algo = "sha256"

# optional overridden filename.
# Default is "/opt/kumomta/etc/dkim/DOMAIN/SELECTOR.key"
filename = "/full/path/to/key."
```

This approach allows for full control of DKIM signing without the need to implement a data integration or fully code the signing parameters in Lua.

## Current KumoMTA Policy Helpers

All policy helpers listed below are implemented in the [Example Lua Policy](./example.md) and can be used to simplify your KumoMTA installation. In addition, the policy helper source code is available to use as a starting point for developing your own integrated configuration: where the helpers are pulling from a TOML or JSON file, they could be modified to connect directly to a data source.

* [Listener_Domains](./domains.md#using-the-listener_domainslua-policy-helper) - Helper for configuring which domains are allowed to relay, process bounces, and process abuse reports.
* [Sources](./sendingips.md#using-the-sourceslua-policy-helper) - Helper for configuring the egress sources and pools used for relaying messages.
* [Queues](./queuemanagement.md#using-the-queues-helper) - Helper for configuring tenant and queue configuration, including retry intervals, tenant identifier headers, and the mapping from tenant to egress pool.
* [Shaping](./trafficshaping.md#using-the-shapinglua-helper) - Helper for configuring traffic shaping rules to use for destination domains. Also can be configured for [Traffic Shaping Automation](./trafficshapingautomation.md).
* [Dkim_Sign](./dkim.md#using-the-dkim_signlua-policy-helper) - Helper for configuring parameters for DKIM signing for each signing domain.
* [Log_Hooks](../operation/webhooks.md#using-the-log_hookslua-helper) - Helper for configuring webhooks.
