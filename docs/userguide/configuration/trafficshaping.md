# Configuring Traffic Shaping

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

The current version of the shaping.lua file is available at [https://github.com/KumoCorp/kumomta/blob/main/assets/shaping.lua](https://github.com/KumoCorp/kumomta/blob/main/assets/shaping.lua). It should be written to *`/opt/kumomta/etc/shaping.lua`*.

The shaping.lua policy reads a JSON file, which needs to be written to *`/opt/kumomta/etc/shaping.json`*, and which is structured as follows:

```json
{
    "default": {
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

As a special case, the domain can be named *default*, in which case those settings will apply globally. The global settings are superseded by the domain settings, which are superseded by the source settings.

By default, shaping.lua treats each domain entry as applying to the site_name for the domain named, and those settings apply to any destination domain that maps to the site. If you need to explicitly override a setting for a destination domain that is not self-hosted but instead part of another site_name, you need to set the `mx_rollup` option to **false** when configuring the domain, as in the example above for foo.com. If you configure a domain that belongs to another configured site without setting the `mx_rollup` option to **false**, you will cause an error.

In the example above, traffic to foo.com will ultimately be as follows:

```json
"connection_limit": 3,
"max_deliveries_per_connection": 5,
"max_connection_rate": "20/min",
"max_message_rate": "5/min",
```

The connection_limit is defined in the default, the max_connection_rate is configured in the default, but is overridden by the yahoo.com site. The max_message_rate is defined in the yahoo.com site, but is in turn overridden by the foo.com domain specific configuration.


The *mx_rollup* option indicates whether or not the settings should apply to the domain or to the site_name. In the example above, even though foo.com is hosted by Yahoo! we want to override the message throttle for the foo.com domain.

The full set of available options is listed in the [kumo.make_egress_path](../../reference/kumo/make_egress_path.md) page of the Reference Manual.
