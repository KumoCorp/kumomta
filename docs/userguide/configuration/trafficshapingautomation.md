# Configuring Traffic Shaping Automation

{{since('2023.08.22-4d895015')}}

Many of the largest MailBox Providers (MBPs) operate platforms that provide feedback to senders through their response codes during the SMTP conversation. To ensure optimum throughput and deliverability, KumoMTA features Traffic Shaping Automation (TSA), via a daemon that monitors responses from the MBPs and adjusts traffic shaping rules on a granular level to ensure compliance with the guidelines of the MBPs in realtime.

## TSA Architecture

To better support clustered installations, as well as to limit load on the primary `kumod` service, Traffic Shaping Automation is implemented via a standalone daemon called *`kumo-tsa-daemon.service`* that is installed as a service that starts automatically when its configuration is present.

The daemon monitors the events recorded by `kumod` and makes adjustments according to user-defined rules.

There are three configuration locations required to implement TSA:

* `tsa_init.lua` - A configuration file that controls the behavior of the TSA daemon.
* `init.lua` - The server's init.lua must be modified in order to properly interact with the TSA daemon.
* `shaping.toml` - A special automation entry is added to domain rules to power TSA adjustments.

## Configuring the `tsa_init.lua` File

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
    -- This is the default file used by the shaping helper
    -- in KumoMTA, which references the community shaping rules
    '/opt/kumomta/share/policy-extras/shaping.toml',

    -- and maybe you have your own rules
    '/opt/kumomta/etc/policy/shaping.toml',
  }
  return shaping
end)
```

!!!warning
    Do not edit the `/opt/kumomta/share/policy-extras/shaping.toml` as it is overwritten when upgrading KumoMTA. Instead, create the `/opt/kumomta/etc/policy/shaping.toml` file as listed above and populate it with your own override rules.

## Changes to the `init.lua` File

!!!note
    It's easiest to reference the [Example Config](./example.md) to see how the complete configuration looks.

The server's `init.lua` file will require modifications to enable it to be used with TSA.

First, the following should be added to the start of the `init.lua` file, just below the initial `local kumo = require 'kumo'` line:

```lua
local shaping = require 'policy-extras.shaping'

local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
}
```

This section enabled communication with the TSA daemon. The publish and subscribe URLs correspond to the TSA daemon's HTTP listener endpoint defined in its tsa_init.lua.  For a single node deployment the values shown here are sufficient.  You may list multiple publish and/or subscribe endpoints to publish to multiple hosts and read shaping configuration from multiple hosts, respectively. In addition, while the `setup_with_automation` call is aware of the community shaping rules file, any custom file must be identified in the `extra_files` directive as seen in the example above.

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

## Changes to the `shaping.toml` File

In addition to the domain-level traffic shaping rules currently in your `shaping.toml` file, you add additional automation entries based on the event you are targeting.

```toml
[["example.com".automation]]
regex = "250 2\\.0\\.0 Ok"
# There is no "trigger" here, so the action is taken immediately
# when a log record matches.
# This action causes delivery to be suspended for the pathway (source + site-name)
action = "Suspend"
# for 2 hours
duration = "2 hours"

[["example.com".automation]]
regex = "250 2\\.0\\.0 boop"
# sets max_connection_rate="100/s"
action = {SetConfig={name="max_connection_rate", value="100/s"}}
# if we see 2 or more matches in an hour. Unlike throttles, this
# doesn't divide down to per-second rates.
trigger = {Threshold="2/hr"}
# The config override will last for 2 hours
duration = "2 hours"
```

The TSA daemon has two actions: temporary suspension of traffic to the triggering combination of egress source and site name, and adjustment of the traffic shaping rules to the triggering combination of egress source and site name.

For more information, see the [Traffic Shaping Automation Rules](../../reference/kumo.shaping/load.md#traffic-shaping-automation-rules) page in the Reference Manual.

## Monitoring the TSA Daemon

Adjustments to the traffic shaping rules are achieved by creating a custom `shaping.toml` file that is maintained by the TSA daemon and loaded as an overlay on the existing `shaping.toml file created by the user.

The generated TOML can be monitored by making an HTTP request. One example using curl:

```console
$ curl -s 'http://localhost:8008/get_config_v1/shaping.toml'
# Generated by tsa-daemon
# Number of entries: 0
```

This call returns the current set of shaping rules in the same format as shaping.toml, the example is of an empty set.

## Debugging Tips
If the tsa-deamon does not appear to be working, you can check to see if it is running with 'sudo systemctl status kumo-tsa-daemon' which should return a message that includes "active (running)".  If not you can stop and start it in a similar way.

```bash
sudo systemctl stop kumo-tsa-daemon
sudo systemctl start kumo-tsa-daemon
```

Data for the TSA daemon is just like any other message in KumoMTA and will follow the same retry rules. The default is to retry in 20 minutes with exponential fallback.  If desired, this (or any other) scheduled queue can be customized with the [get_queue_config](https://docs.kumomta.com/reference/events/get_queue_config/) hook.
