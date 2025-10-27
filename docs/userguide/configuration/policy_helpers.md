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
local dkim_signer = dkim_sign:setup {
  files = { '/opt/kumomta/etc/policy/dkim_data.toml' },
}

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

{% call toml_data() %}
[domain."example.com"]
selector = 'dkim1024'
headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]
algo = "sha256"

# optional overridden filename.
# Default is "/opt/kumomta/etc/dkim/DOMAIN/SELECTOR.key"
filename = "/full/path/to/key."
{% endcall %}

This approach allows for full control of DKIM signing without the need to implement a data integration or fully code the signing parameters in Lua.

## Current KumoMTA Policy Helpers

All policy helpers listed below are implemented in the [Example Lua Policy](./example.md) and can be used to simplify your KumoMTA installation. In addition, the policy helper source code is available to use as a starting point for developing your own integrated configuration: where the helpers are pulling from a TOML or JSON file, they could be modified to connect directly to a data source.

* [Listener_Domains](./domains.md#using-the-listener_domainslua-policy-helper) - Helper for configuring which domains are allowed to relay, process bounces, and process abuse reports.
* [Sources](./sendingips.md#using-the-sourceslua-policy-helper) - Helper for configuring the egress sources and pools used for relaying messages.
* [Queues](./queuemanagement.md#using-the-queues-helper) - Helper for configuring tenant and queue configuration, including retry intervals, tenant identifier headers, and the mapping from tenant to egress pool.
* [Shaping](./trafficshaping.md#using-the-shapinglua-helper) - Helper for configuring traffic shaping rules to use for destination domains. Also can be configured for [Traffic Shaping Automation](trafficshaping.md).
* [Dkim_Sign](./dkim.md#using-the-dkim_signlua-policy-helper) - Helper for configuring parameters for DKIM signing for each signing domain.
* [Log_Hooks](../operation/webhooks.md#using-the-log_hookslua-helper) - Helper for configuring webhooks.

## Validating Your Configuration

{{since('2024.09.02-c5476b89')}}

You can perform a deep validation on your configuration before you deploy it by
running `kumod` in validation mode:

```console
$ /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --validate
```

You can safely run this concurrently with an active `kumod` service; they will
not conflict with each other.

When run in this mode, the various helpers that you have enabled will perform
deep *referential integrity* checks, as well as some other extended validations that
are not normally performed when the underlying configuration files are refreshed.
The sorts of checks performed include the following:

   * `shaping` - any warnings reported by the underlying rust code
      will be reported here and cause validation to fail. This is
      functionally equivalent to using the `validate-shaping` binary,
      except that it will automatically be passed the set of shaping
      files defined by your `init.lua`

      If the `sources` helper is also configured, the list of sources
      referenced by the shaping config will be cross-checked against
      the sources data to confirm that all possible sources are defined.

   * `sources` - each listed source and pool will be validated by
      calling `kumo.make_egress_source` or `kumo.make_egress_pool`
      respectively.

      Pool membership will be validated to confirm that every
      listed pool is defined in the sources data.

   * `queues` - each domain and tenant that references an `egress_pool`
      will be cross-checked with the `sources` helper, if the sources
      helper has been configured.

   * `dkim` - a dummy message is created and signed for each configured
      domain, before being discarded, allowing errors in the configuration to
      be detected.  An additional dummy message is created that doesn't match
      any configured domain to test additional signature blocks.
