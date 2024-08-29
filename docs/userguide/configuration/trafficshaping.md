# Configuring Traffic Shaping

<iframe width="560" height="315" src="https://www.youtube.com/embed/Vxbe5ExMOXk?si=2SC7o8FObyvWqavl" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

By default, the KumoMTA server will send messages in the Ready Queue as fast as possible, with unlimited messages per connection. Because each Mailbox Provider (MBP) has their own expectations around how remote hosts should behave, best practices require that a sender employ a number of different traffic shaping throttles dependent on the destination provider and the reputation of the source IP.

When KumoMTA needs to connect to a remote host to deliver messages, the [get_egress_path_config](../../reference/events/get_egress_path_config.md) is fired in order to define what configuration to use for the path.

```lua
kumo.on('get_egress_path_config', function(domain, egress_source, site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
  }
end)
```

The [kumo.make_egress_path](../../reference/kumo/make_egress_path/index.md) function is called with the relevant parameters for the connection, determined by the Domain, Egress Source, and Site Name.

Where the Domain in the event call is the actual domain from the recipient address (for example, corp.com for a message destined to user@corp.com), the site_name is defined in the server by building a string representation of all MX servers that service the destination domain.

The site_name concept allows managing traffic more effectively for domains that have several or even a great many aliases. Rather than treating each domain as a separate destination, the traffic can be grouped and managed more closely to how the receiving site sees it: as one source.

Many of the largest Mailbox Providers handle multiple domains, so queueing by site_name allows those organizations to be treated as a single entity for queueing and traffic shaping rather than as a large collection of single domains. For example, if all Gmail hosted mail was queued and throttled based on destination domain, an MTA would make thousands of excess connections to the Gmail servers, because it would consider every domain that routes to the Gmail servers as a separate and distinct entity with its own servers, potentially resulting in failures and throttling. Instead, KumoMTA assigns the same site_name to each message destined for a Gmail-hosted domain because the group of MX servers is the same.

Messages in the Ready Queue are grouped into separate queues based on the combination of egress_source and site_name. The domain is provided for convenience when working on what parameters to use.

## Using The Shaping.lua Helper

While you can use whatever Lua policy you see fit to determine which traffic-shaping rules you wish to use for sending, the shaping.lua policy file is available as both an example of how rules can be stored and retrieved from a data source, and as a default set of traffic-shaping rules.

To use the shaping.lua helper, add the following to your init.lua, outside the init event:

```lua
local shaping = require 'policy-extras.shaping'
kumo.on('get_egress_path_config', shaping:setup())
```

The shaping.lua policy reads a TOML file that is maintained by the community and included in all repos, also found at [https://github.com/KumoCorp/kumomta/blob/main/assets/policy-extras/shaping.toml](https://github.com/KumoCorp/kumomta/blob/main/assets/policy-extras/shaping.toml), and which is structured as follows:

{% call toml_data() %}
["default"]
connection_limit = 10
max_connection_rate = "100/min"
max_deliveries_per_connection = 100
max_message_rate = "100/s"
idle_timeout = "60s"
enable_tls = "Opportunistic"
consecutive_connection_failures_before_delay = 100

["example.com"]
mx_rollup = false
max_deliveries_per_connection = 100
connection_limit = 3
{% endcall %}

As a special case, the domain can be named *default*, in which case those settings will apply globally. The global settings are superseded by the domain settings, which are superseded by the source settings.

The full set of available options is listed in the [kumo.make_egress_path](../../reference/kumo/make_egress_path/index.md) page of the Reference Manual.

### MX Rollups and Option Inheritance

By default, shaping.lua treats each domain entry as applying to the site_name for the domain named, and those settings apply to any destination domain that maps to the site. If you need to explicitly override a setting for a destination domain that is not self-hosted but instead part of another site_name, you need to set the `mx_rollup` option to **false** when configuring the domain, as in the example above for *example.com.* If you configure a domain that belongs to another configured site without setting the `mx_rollup` option to **false**, you will cause an error.

Consider the following example, with foo.com being a domain hosted by Yahoo!:

{% call toml_data() %}
["default"]
connection_limit = 10
max_connection_rate = "100/min"
max_deliveries_per_connection = 100
max_message_rate = "100/s"
idle_timeout = "60s"
enable_tls = "Opportunistic"
consecutive_connection_failures_before_delay = 100

["yahoo.com"]
max_deliveries_per_connection = 20

["foo.com"]
mx_rollup = false
max_deliveries_per_connection = 50
connection_limit = 3

["foo.com".sources."IP-1"]
max_deliveries_per_connection = 5
{% endcall %}

This example would result in the following active settings for mail being sent to foo.com on the IP-1 Egress Source:

{% call toml_data() %}
connection_limit = 3
max_deliveries_per_connection = 5
max_connection_rate = "100/min"
max_message_rate = "100/s"
{% endcall %}

The *mx_rollup* option indicates whether or not the settings should apply to the domain or the site_name. In the example above, even though foo.com is hosted by Yahoo! we want to override the message throttle for the foo.com domain. The mx_rollup option is true by default and only needs to be specified for domains that override the main site name entry.

While the default max_deliveries_per_connection is 100, it is overridden for yahoo.com (and all domains that share the same site name as the yahoo.com domain) to 20. The foo.com domain is part of the same site name as yahoo.com, but because mx_rollup is set to false the foo.com domain is treated separately and instead is set to 50. Because there is a sources entry for IP-1, the max_deliveries_per_connection is further overridden to 5 for that source's traffic in particular.

### Pattern Matching Rollups

{{since('dev')}}

There are a number of mailbox providers for which the default MX-based rollup
scheme cannot be used because their MX records and infrastructure are
distributed in such a way that the automatic MX grouping is not effective when
it comes to shaping traffic across that infrastructure.

For those situations is is desirable to adopt hostname based pattern matching
and employ connection limits and message rate throttles for all destination hosts
that match.

You can configure this using a `provide` block in your shaping file(s).

For an example, let's consider Outlook and Hotmail. They are both run by the
same provider and backend, but their domain names are very different, and their
MX hostnames are also both different from each other, so the normal MX-based
rollup is not effective:

```console
dig +short mx hotmail.com
2 hotmail-com.olc.protection.outlook.com.
dig +short mx outlook.com
5 outlook-com.olc.protection.outlook.com.
```

However, we can see that the individual MX hostnames have the same
`.olc.protection.outlook.com` suffix, so we can use that to identify this
provider:

{% call toml_data() %}
[provider."Office 365"]
# Every domain whose MX hostnames ALL have .old.protection.outlook.com will
# match this provider block
match=[{MXSuffix=".olc.protection.outlook.com"}]
# Let's require TLS for this provider
enable_tls = "Required"
# And set a provider-specific connection limit and message rate
provider_connection_limit = 10
provider_max_message_rate = "120/s"
{% endcall %}

Now, messages destined for either `hotmail.com` or `outlook.com`, or any
other domain whose MX host names all have the suffix `.olc.protection.outlook.com`,
will match the provider block and have the options defined there applied.

The `match` field is an array and can list multiple match candidates. A provider
block matches if *any* of the `match` elements matches.

The match can be one of two possible options:

* `{MXSuffix="SUFFIX"}` - matches if ALL of the individual MX hostname suffixes
  match the specified suffix string.
* `{DomainSuffix="SUFFIX"}` - matches if the domain name suffix matches the
  specified suffix string.

### Shaping Option Resolution Order and Precedence

When resolving the configuration for a site, the options are resolved in the
following order:

1. The values for the `default` domain block are taken as the base
2. Any matching `provider` blocks are then merged in
3. Any matching `provider` + `source` blocks for the current source are merged in
4. Any matching *site name* blocks are merged in. These are domain blocks that have the default (implied) or explicitly configured `mx_rollup = true` option set in them.
5. Any matching domain blocks are merged in. These are domain blocks that have `mx_rollup=false` set in them.
6. Any matching *site name* + `source` blocks are merged.
7. Any matching domain + `source` blocks are merged.

Within any of these steps above, the options are merged in the order that they
appear across your configuration files, so the most recently specified value
will take precedence overall.

You can specify `replace_base=true` in a block to have that block override the
current set of accumulated values.

Most options merge directly over the top of earlier options, but the
[additional_connection_limits](../../reference/kumo/make_egress_path/additional_connection_limits.md) and
[additional_message_rate_throttles](../../reference/kumo/make_egress_path/additional_message_rate_throttles.md)
options merge the maps together.

### Overriding the shaping.toml File

The `shaping.toml` file provides a community-contributed collection of traffic shaping rules that are useful for new servers, but traffic shaping rules are often configured in the context of the reputation of the various domains and IP addresses in a given environment, making it necessary to customize the rules according to your specific use cases.

In addition, all per-source traffic shaping options must be in a user-defined shaping file since the default shaping.toml file does not suppoort per-source configuration.

Because the shaping.toml file is part of the install repository, it should not be modified. Instead, create a separate file with your own traffic shaping rules in either TOML or JSON formats and pass it as part of the call to `shaping:setup()`:

```lua
-- load the community shaping.toml + local settings
kumo.on(
  'get_egress_path_config',
  shaping:setup { '/opt/kumomta/etc/shaping.json' }
)
```

You can load multiple override files too if you wish, by adding each file name to that table:

```lua
-- load the community shaping.toml + local settings
kumo.on(
  'get_egress_path_config',
  shaping:setup {
    '/opt/kumomta/etc/shaping.toml',
    '/opt/kumomta/etc/shaping-generated.json',
  }
)
```

When creating an override file, any settings that overlap with an existing domain definition will append any existing settings in the `shaping.toml` file, replacing any directly overlapping options.

If you wish to discard all existing options for a domain defined in the shaping.toml file, add the replace_base = true option for that domain. The following example will replace the existing default traffic shaping options, but augment those for the other defined domains:

```json
{
    "default": {
        "replace_base": true
        "connection_limit": 3,
        "max_connection_rate": "100/min",
    },
    "yahoo.com": {
        "max_deliveries_per_connection": 5,
        "max_connection_rate": "20/min",
        "max_message_rate": "50/s",
        }
    },
    "foo.com": {
        "mx_rollup": false // foo.com is hosted by yahoo.com, but we want to throttle it specifically.
        "max_message_rate": "5/min",
    },
    "gmail.com": {
        "connection_limit": 3,
        "sources": {
            "ip-1": {
                "connection_limit": 5
            },
        },
    },
}
```

### Testing your shaping file

Included in the standard deployment is a validation tool for testing the syntax of your shaping.toml override file. The file located at `/opt/kumomta/sbin/validate-shaping` can be used to validate the syntax of your shaping file.  If there are no errors, it will return an "OK".
```bash
$ /opt/kumomta/sbin/validate-shaping /opt/kumomta/etc/policy/shaping.toml
OK
```

## Automating Traffic Shaping

This section has covered how to configure traffic shaping in a static manner, but many traffic shaping decisions require real-time adjustments. See the [Configuring Traffic Shaping Automation](./trafficshapingautomation.md) page for more information.
