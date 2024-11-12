# Configuring Traffic Shaping

<iframe width="560" height="315" src="https://www.youtube.com/embed/Vxbe5ExMOXk?si=2SC7o8FObyvWqavl" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

By default, the KumoMTA server will deliver messages as quickly as possible to each destination, with few restrictions regarding number of connections, number of messages per connection, or number of messages per second. Because unthrottled sending is unwelcome by most MailBox Providers (MBPs) it is highly recommended that KumoMTA users implement traffic shaping in order to limit sending speeds to something more aligned with the expectations of the individual MBPs.

Common throttles include concurrent connection limits, messages per connection, rate of opening connections, and rate of sending messages. In addition, users can set options for various timeouts, queue sizes, and what encryption rules to use when communicating with remote hosts.

## Scoping Traffic Shaping Rules

When KumoMTA needs to connect to a remote host to deliver messages, the [get_egress_path_config](../../reference/events/get_egress_path_config.md) event is fired in order to determine what configuration to use for that specific pathway.

```lua
kumo.on('get_egress_path_config', function(domain, egress_source, site_name)
  return kumo.make_egress_path {
    enable_tls = 'OpportunisticInsecure',
  }
end)
```

The event passes three attributes that are keyed to determine the desired traffic shaping rules, namely the `domain`, `egress_source`, and `site_name`.

Where the `domain` in the event call is the actual domain from the recipient address (for example, corp.com for a message destined to `user@corp.com`), the site_name is defined in the server by building an expression that represents all MX servers returned in an MX lookup for the `domain`. For example, if `user@corp.com` is hosted by Google Workspace then the `site_name` might be something similar to `(alt1|alt2|alt3|alt4)?.aspmx.l.google.com`.

The site_name concept allows managing traffic more effectively for MBPs that host a large number of domains. Rather than treating each domain as a separate destination with its own queues and traffic shaping counters, the traffic can be grouped and shaped as a single MX group, which is what the MBPs expect when they receive incoming traffic.

For example: if a sender wanted to limit connections to 10 per domain, and Google Workspace hosted 1,000 domains in the sender's queues, a server without MX rollup or sitenames would open 10,000 connections (1000 domains * 10 connections each). With the use of `site_name`, KumoMTA merges the 1,000 domains under a single `site_name` and maintains the limit of 10 connections per `egress_source` to the Google Workspace servers.

Messages in the Ready Queue are grouped into separate queues based on the combination of egress_source and site_name. The domain is provided for convenience when working out what parameters to use.

## The `shaping.lua` Helper

While users are free to implement their traffic shaping rules as they see fit, the KumoMTA team has provided the `shaping.lua` helper as a pre-built implementation based on static configuration files in either JSON or TOML format, with support for various configuration scopes and automated rules using Traffic Shaping Automation.

The remainder of this section of the documentation is focused on the use of the `shaping.lua` helper.

## Traffic Shaping Automation

Many of the largest MailBox Providers (MBPs) operate platforms that provide feedback to senders through their response codes during the SMTP conversation. This feedback will include information related to the traffic shaping patterns in use by the sending including bounces for too many connections, too many messages per connection, sending rate, and sender reputation.

To ensure optimum throughput and deliverability, KumoMTA features Traffic Shaping Automation (TSA) that monitors responses from the MBPs and adjusts traffic shaping rules on a granular level to ensure compliance with the guidelines of the MBPs in realtime.

### TSA Architecture

To better support clustered installations, as well as to limit load on the primary `kumod` service, Traffic Shaping Automation is implemented via a standalone daemon called the *`kumo-tsa-daemon.service`* that starts automatically when its configuration is present.

The daemon monitors the events sent to it by the KumoMTA `kumod` process and instructs the `kumod` process to make adjustments to its traffic shaping rules according to user-defined actions.

There are three configuration locations required to implement TSA:

* `init.lua` - The server's init.lua must be modified in order to properly interact with the TSA daemon.
* `tsa_init.lua` - A configuration file that controls the behavior of the TSA daemon.
* `shaping.toml` - A special automation entry is added to domain rules to power TSA adjustments.

## Using The Shaping.lua Helper

The following steps must be followed to configure traffic shaping in KumoMTA.

### Set Up Traffic Shaping Rules Files

The `shaping.lua` policy reads one or more configuration files in either TOML or JSON formats.

#### The Default shaping.toml File

By default, `shaping.lua` reads a `shaping.toml` file maintained by the KumoMTA team and included in all repos, available at [https://github.com/KumoCorp/kumomta/blob/main/assets/policy-extras/shaping.toml](https://github.com/KumoCorp/kumomta/blob/main/assets/policy-extras/shaping.toml), and which is structured as follows:

{% call toml_data() %}
["default"]
connection_limit = 10
max_connection_rate = "100/min"
max_deliveries_per_connection = 100
max_message_rate = "100/s"
idle_timeout = "60s"
enable_tls = "Opportunistic"
consecutive_connection_failures_before_delay = 100

[provider."google"]
match=[
        {MXSuffix=".google.com"},
        {MXSuffix=".googlemail.com"}
]
max_deliveries_per_connection = 50
provider_connection_limit = 5
consecutive_connection_failures_before_delay = 5

[["gmail.com".automation]]
regex = "This message does not have authentication information"
action = "SuspendTenant"
duration = "3 hours"

[provider."yahoo"]
match=[{MXSuffix=".yahoodns.net"}]
max_deliveries_per_connection = 20

[[provider."yahoo".automation]]
regex = "\\[TS04\\]"
action = "Suspend"
duration = "2 hours"

["comcast.net"]
connection_limit = 25
max_deliveries_per_connection = 1000
enable_tls = "Required"
idle_timeout = "30s"
consecutive_connection_failures_before_delay = 24

[["comcast.net".automation]]
regex = "RL0000"
# sets max_connection_rate="10,000 per hour"
action = {SetConfig={name="max_connection_rate", value="10000/h"}}
# if we see 2 or more matches in an hour. Unlike throttles, this
# doesn't divide down to per-second rates.
trigger = {Threshold="2/hr"}
# The config override will last for 2 hours
duration = "2 hours"
{% endcall %}

As a special case, the domain can be named *default*, in which case those settings will apply globally. The global settings are superseded by the domain settings, which are superseded by the source settings.

The full set of available options is listed in the [kumo.make_egress_path](../../reference/kumo/make_egress_path/index.md) page of the Reference Manual.

The full set of Traffic Shaping Automation actions is available on the [traffic shaping](../../reference/kumo.shaping/load.md) page of the Reference Manual.

#### The Community shaping.toml File

In addition, the KumoMTA Github repo includes a traffic shaping rules file maintained by the community, available at [https://github.com/KumoCorp/kumomta/blob/main/assets/community/shaping.toml](https://github.com/KumoCorp/kumomta/blob/main/assets/community/shaping.toml) that can also be loaded explicitly as an additional resource for users.

#### Custom Shaping Files

Finally, users can opt to create their own shaping rules file that can augment or replace the rules defined in the previous two files.

If you intend to manually maintain your own shaping rules, we recommend using TOML as your file format, whereas if you intend to automatically maintain your traffic shaping rules, we recommend using JSON as your file format. Both example formats are listed above.

While you can place a custom shaping file at any readable location, it is common to place the file at `/opt/kumomta/etc/policy/shaping.[toml|json]` for consistency with examples used elsewhere in the documentation.

#### Order of Definition and Configuration File Precedence

The order in which your traffic shaping configuration files are defined affects how they are loaded and in turn which options are preserved when there is a conflict between the files.

The `shaping.lua` helper reads files sequentially, and the most recently defined file will overwrite any option set by a previously defined file.

Because of this, we recommend setting up shaping following this pattern, which will be explained later in this page:

```lua
local shaping = require 'policy-extras.shaping'

local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = {
    '/opt/kumomta/share/policy-extras/shaping.toml',
    '/opt/kumomta/share/community/shaping.toml',
    '/opt/kumomta/etc/policy/shaping_custom.toml',
  },
}
```

!!!Note
    When a given scope is defined in multiple files, the more recently read file does not completely replace the configuration defined in the previous file, instead the options within that scope are merged.

If you want to completely replace the information for a given block, you
can indicate that by using `replace_base = true`:

{% call toml_data() %}
["gmail.com"]
# Discard any other `gmail.com` rules provided by earlier files
replace_base = true
connection_limit = 10
{% endcall %}

`replace_base` is only meaningful in the context of the current domain section
in the current file; subsequent sections for that same domain will continue
to merge in as normal, unless they also use `replace_base`.

#### MX Rollups and Option Inheritance

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

#### Pattern Matching Rollups

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

The match can be one of two possible options:

* `{MXSuffix="SUFFIX"}` - matches if ALL of the individual MX hostname suffixes
  match the specified suffix string.
* `{DomainSuffix="SUFFIX"}` - matches if the domain name suffix matches the
  specified suffix string.

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

#### Shaping Option Resolution Order and Precedence

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

!!!warning
    There is currently no mechanism for unsetting an option previously merged in. If there is a throttle set earlier (for example in `[default]`) that you wish to unset rather than explicitly define a different throttle then you **must** use `replace_base=true` to replace all previously merged options.

Most options merge directly over the top of earlier options, but the
[additional_connection_limits](../../reference/kumo/make_egress_path/additional_connection_limits.md) and
[additional_message_rate_throttles](../../reference/kumo/make_egress_path/additional_message_rate_throttles.md)
options merge the maps together.

#### Writing Your Own Traffic Shaping Rules

The `/opt/kumomta/share/policy-extras/shaping.toml` file provides a collection of traffic shaping rules provided by the KumoMTA team that are useful for new servers. In addition, a community-maintained set of traffic shaping rules is available at `/opt/kumomta/share/community/shaping.toml`.

The files listed above are maintained within the KumoMTA GitHub repository and are updated with each release, meaning that any local edits to these files will be lost any time the KumoMTA install is updated.

In addition, neither of these files are all-encompassing, you will likely encounter scenarios that require you to implement your own logic, either to address your specific reputation or to reflect specialized knowledge you have gained.

To maintain your own traffic shaping rules, create a separate file with your own traffic shaping rules in either TOML or JSON formats, typically called `/opt/kumomta/etc/policy/custom-shaping.[toml|json]` and pass it as part of the call to set up traffic shaping.

### Test Your Shaping Files

Included in the standard deployment is a validation tool for testing the syntax of your shaping.toml override file. The file located at `/opt/kumomta/sbin/validate-shaping` can be used to validate the syntax of your shaping file.  If there are no errors, it will return an "OK".

```bash
$ /opt/kumomta/sbin/validate-shaping /opt/kumomta/etc/policy/custom-shaping.toml
OK
```

### Configure Traffic Shaping In Your `init.lua` Server Policy

!!!note
    It's easiest to reference the [Example Config](./example.md) to see how the complete configuration looks.

The server's `init.lua` file will require modifications to enable it to be used with TSA.

First, the following should be added to the start of the `init.lua` file, just below the initial `local kumo = require 'kumo'` line:

```lua
local shaping = require 'policy-extras.shaping'

local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = {
    '/opt/kumomta/share/policy-extras/shaping.toml',
    '/opt/kumomta/share/community/shaping.toml',
    '/opt/kumomta/etc/policy/shaping_custom.toml',
  },
}
```

This section enabled communication with the TSA daemon. The publish and subscribe URLs correspond to the TSA daemon's HTTP listener endpoint defined in its tsa_init.lua.  For a single node deployment the values shown here are sufficient.  You may list multiple publish and/or subscribe endpoints to publish to multiple hosts and read shaping configuration from multiple hosts, respectively. In addition, while the `setup_with_automation` call is aware of the community shaping rules file, any custom file must be identified in the `extra_files` directive as seen in the example above.

!!!warning
    As mentioned previously, your rules merge with the other files listed unless a given block has `replace_base=true`. To fully remove the defaults provided by the KumoMTA team you need the following:

    ```lua
    local shaping = require 'policy-extras.shaping'

    local shaper = shaping:setup_with_automation {
      publish = { 'http://127.0.0.1:8008' },
      subscribe = { 'http://127.0.0.1:8008' },
      no_default_files=true,
      extra_files = { 
            '/opt/kumomta/share/community/shaping.toml`, 
            '/opt/kumomta/etc/policy/shaping_custom.toml',
            },
    }
    ```

Next, the following should be added within the `kumo.on('init', function()` block:

```lua
-- Configure publishing of logs to automation daemon
shaper.setup_publish()
```

This enables the logging required by the TSA daemon.

Finally, the following must be added outside the init event to enable the TSA manipulations:

```lua
-- Attach various hooks to the shaper
kumo.on('get_egress_path_config', shaper.get_egress_path_config)
```

### Configure The `tsa_init.lua` File

The `tsa_init.lua` file controls the behavior of the TSA daemon, and should be written to `/opt/kumomta/etc/policy/tsa_init.lua`.

An example of the `tsa_init.lua` file is below:

```lua
local tsa = require 'tsa'
local kumo = require 'kumo'

kumo.on('tsa_init', function()
  tsa.start_http_listener {
    listen = '0.0.0.0:8008',
    trusted_hosts = { '127.0.0.1', '::1' },
  }
end)

local cached_load_shaping_data = kumo.memoize(kumo.shaping.load, {
  name = 'tsa_load_shaping_data',
  ttl = '5 minutes',
  capacity = 4,
})

kumo.on('tsa_load_shaping_data', function()
  local shaping = cached_load_shaping_data {
    '/opt/kumomta/share/policy-extras/shaping.toml',
    '/opt/kumomta/share/community/shaping.toml',
    '/opt/kumomta/etc/policy/custom-shaping.toml',
  }
  return shaping
end)
```

!!!note
    The `tsa_init.lua` has no implicit loading of the default `shaping.toml` file. To avoid loading the default file simply omit it.

### Monitoring the TSA Daemon

Adjustments to the traffic shaping rules are achieved by creating a custom `shaping.toml` file that is maintained by the TSA daemon and loaded as an overlay on the existing `shaping.toml file created by the user.

The generated TOML can be monitored by making an HTTP request. One example using curl:

```console
$ curl -s 'http://localhost:8008/get_config_v1/shaping.toml'
# Generated by tsa-daemon
# Number of entries: 0
```

This call returns the current set of shaping rules in the same format as shaping.toml, the example is of an empty set.

### Debugging Tips
If the tsa-deamon does not appear to be working, you can check to see if it is running with `sudo systemctl status kumo-tsa-daemon` which should return a message that includes "active (running)".  If not you can stop and start it in a similar way.

```bash
sudo systemctl stop kumo-tsa-daemon
sudo systemctl start kumo-tsa-daemon
```

Another way to identify that the TSA daemon is running is to use its API endpoint with curl:

```bash
curl -s 'http://localhost:8008/get_config_v1/shaping.toml' | head
# Generated by tsa-daemon
# Number of entries: 2576
```

Data being sent to the TSA daemon is handled the same as any other message in KumoMTA and will follow the same retry rules. The default is to retry in 20 minutes with exponential fallback.  If desired, this (or any other) scheduled queue can be customized with the [get_queue_config](https://docs.kumomta.com/reference/events/get_queue_config/) hook or in your shaping.toml file.

### Clustering

There are special considerations when implementing traffic shaping in a clustered environment, see the [Clustering Chapter](../clustering/index.md) for more information.
