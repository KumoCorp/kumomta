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

The [kumo.make_egress_path](../../reference/kumo/make_egress_path.md) function is called with the relevant parameters for the connection, determined by the Domain, Egress Source, and Site Name.

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

```toml
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
```

As a special case, the domain can be named *default*, in which case those settings will apply globally. The global settings are superseded by the domain settings, which are superseded by the source settings.

The full set of available options is listed in the [kumo.make_egress_path](../../reference/kumo/make_egress_path.md) page of the Reference Manual.

### MX Rollups and Option Inheritance

By default, shaping.lua treats each domain entry as applying to the site_name for the domain named, and those settings apply to any destination domain that maps to the site. If you need to explicitly override a setting for a destination domain that is not self-hosted but instead part of another site_name, you need to set the `mx_rollup` option to **false** when configuring the domain, as in the example above for *example.com.* If you configure a domain that belongs to another configured site without setting the `mx_rollup` option to **false**, you will cause an error.

Consider the following example, with foo.com being a domain hosted by Yahoo!:

```toml
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
```

This example would result in the following active settings for mail being sent to foo.com on the IP-1 Egress Source:

```toml
connection_limit = 3
max_deliveries_per_connection = 5
max_connection_rate = "100/min"
max_message_rate = "100/s"
```

The *mx_rollup* option indicates whether or not the settings should apply to the domain or the site_name. In the example above, even though foo.com is hosted by Yahoo! we want to override the message throttle for the foo.com domain. The mx_rollup option is true by default and only needs to be specified for domains that override the main site name entry.

While the default max_deliveries_per_connection is 100, it is overridden for yahoo.com (and all domains that share the same site name as the yahoo.com domain) to 20. The foo.com domain is part of the same site name as yahoo.com, but because mx_rollup is set to false the foo.com domain is treated separately and instead is set to 50. Because there is a sources entry for IP-1, the max_deliveries_per_connection is further overridden to 5 for that source's traffic in particular.

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

### Tesing your shaping file

Included in the standard deployment is a validation tool for testing the syntax of your shaping.toml override file. The file located at `/opt/kumomta/sbin/validate-shaping` can be used to validate the syntax of your shaping file.  If there are no errors, it will return an "OK".
```bash
$ /opt/kumomta/sbin/validate-shaping /opt/kumomta/etc/policy/shaping.toml
OK
```

## Automating Traffic Shaping

This section has covered how to configure traffic shaping in a static manner, but many traffic shaping decisions require real-time adjustments. See the [Configuring Traffic Shaping Automation](./trafficshapingautomation.md) page for more information.
