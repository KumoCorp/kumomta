# MX Rollups and Provider Blocks

## MX Rollups and Option Inheritance

By default, shaping.lua treats each domain entry as applying to the site_name generated for that domain, and those settings apply to any destination domain that also maps to the site. If you need to explicitly override a setting for a destination domain without consideration for the site_name, you need to set the `mx_rollup` option to **false** when configuring the domain.

If you configure a domain that belongs to a configured site without setting the `mx_rollup` option to **false**, you will cause an error.

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

## Pattern Matching Rollups

{{since('2024.11.08-d383b033')}}

There are a number of mailbox providers that host multiple domains, but which do not provide consistent MX results across all hosted domains.

For those situations is is desirable to adopt pattern matching against the hostnames used by the provider's MX records,
and employ connection limits and message rate throttles for all destination hosts that match.

You can configure this using a `provider` block in your shaping file(s).

For an example, let's consider Microsoft. Microsoft hosts two different consumer email domains (Outlook and Hotmail) as well as Office 365. While the consumer domains are run on the same servers, they have two different MX patterns:

```console
dig +short mx hotmail.com
2 hotmail-com.olc.protection.outlook.com.
dig +short mx outlook.com
5 outlook-com.olc.protection.outlook.com.
```

We can see that the individual MX hostnames have the same
`.olc.protection.outlook.com` suffix, so we can use that to identify the consumer MXes.

In addition, Microsoft has recently announced a change to the MX hostnames used by Office 365, where existing MX records will end in `.mail.protection.outlook.com` but any user who wishes to active DANE to enhance security is to use an MX hostname that ends in `.mx.microsoft`.

To address these three scenarios, we can use the following provider blocks:

{% call toml_data() %}
[provider."outlook"]
match=[{MXSuffix=".olc.protection.outlook.com"}]
max_deliveries_per_connection = 50
provider_connection_limit = 5

[[provider."outlook".automation]]
regex = "temporarily rate limited due to IP reputation"
action = "Suspend"
duration = "1 hour"

[provider."office365"]
match=[{MXSuffix=".mail.protection.outlook.com"}]
max_deliveries_per_connection = 50
provider_connection_limit = 5

[provider."office365-dane"]
match=[{MXSuffix=".mx.microsoft"}]
enable_dane = true
max_deliveries_per_connection = 50
provider_connection_limit = 5
{% endcall %}

Now, messages destined for either `hotmail.com` or `outlook.com`, or any
other domain whose MX host names all have the suffix `.olc.protection.outlook.com`,
will match the `outlook` provider block and have the options defined there applied, including an automation rule, and any Office 365 hosted domain using the new `.mx.microsoft` pattern will have DANE enabled.

The `match` field is an array and can list multiple match candidates. A provider
block matches if *any* of the `match` elements are matched, as seen in this example:

{% call toml_data() %}
[provider."google"]
match=[
        {MXSuffix=".google.com"},
        {MXSuffix=".googlemail.com"}
]
max_deliveries_per_connection = 50
provider_connection_limit = 5
consecutive_connection_failures_before_delay = 5
provider_max_message_rate = "100/s"
{% endcall %}

The match can be one of these possible options:

* `{DomainSuffix="SUFFIX"}` - matches if the domain name suffix matches the
  specified suffix string.
* `{MXSuffix="SUFFIX"}` - matches if one of the MX hostnames matches the
  specified suffix string. (but see below!)
* `{HostName="NAME"}` - matches if one of the MX hostnames exactly equals
  the specified name. (but see below!) {{since('2025.01.23-7273d2bc', inline=True)}}

When matching MX hostnames, rather than DomainSuffixes, every hostname from the
MX record must match one or more of the `MXSuffix` or `HostName` match rules in
order to fully match a destination site against the provider.  The reason for
this is to avoid pathologically weird situations when someone has a vanity
domain that blends multiple different providers together.

!!!note
    The suffix matching is *not* a regex operation, it is purely based on whether the string specified appears at the end of the MX or domain being tested. Do not use any wildcard characters.


The provider block introduces two new options: `provider_connection_limit` and `provider_max_message_rate`. When a provider is defined, it does not merge the various `site_name` queues covered by the provider together, which means that the `connection_limit` and `max_message_rate` options will not be enforced across all matching queues, but will be applied separately to each ready queue covered by the provider block.

When the `provider_connection_limit` and `provider_max_message_rate` options are set, the throttles defined will be enforced across all matching site_name ready queues for that provider. This is typically the desired behavior. One example of a scenario where the provider_ options would not be used is Mimecast: each regional MX pattern used by Mimecast is a separate set of servers in that region, but traffic shaping expectations are the same for all regions. To address this we use a provider block without the `provider_` throttles:

{% call toml_data() %}
[provider."mimecast"]
match=[{MXSuffix=".mimecast.com"},{MXSuffix=".mimecast.co.za"},{MXSuffix=".mimecast-offshore.com"}]
max_deliveries_per_connection = 100
connection_limit = 10
{% endcall %}

In this case we can define traffic shaping rules that apply to Mimecast globally, but which are still enforced by each region's ready queue without limiting worldwide traffic.

!!!note
    Both the `provider_` and regular throttles can be set, where `connection_limit` would be for the individual site names, and `provider_connection_limit` would cap the overall connection count. The same would apply for `max_message_rate` and `provider_max_message_rate`.
