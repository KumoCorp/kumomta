# Traffic Shaping Automation

Many of the largest MailBox Providers (MBPs) operate platforms that provide feedback to senders through their response codes during the SMTP conversation. This feedback will include information related to the traffic shaping patterns in use by the sender, including bounces for too many connections, too many messages per connection, sending rate, and sender reputation.

To ensure optimum throughput and deliverability, KumoMTA features Traffic Shaping Automation (TSA) that monitors responses from the MBPs and adjusts traffic shaping rules on a granular level to ensure compliance with the guidelines of the MBPs in real time.

## TSA Architecture

To better support clustered installations, as well as to limit load on the primary `kumod` service, Traffic Shaping Automation is implemented via a standalone daemon called the *`kumo-tsa-daemon.service`* that starts automatically when its configuration is present.

The daemon monitors the events sent to it by the KumoMTA `kumod` process and instructs the `kumod` process to make adjustments to its traffic shaping rules according to user-defined actions.

There are three configuration locations required to implement TSA:

* `init.lua` - The server's init.lua must be modified in order to properly interact with the TSA daemon.
* `tsa_init.lua` - A configuration file that controls the behavior of the TSA daemon.
* `shaping.toml` - A special automation entry is added to domain rules to power TSA adjustments.

## Configure Traffic Shaping In Your `init.lua` Server Policy

!!! note
    It's easiest to reference the [Example Config](../configuration/example.md) to see how the complete configuration looks.

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
    '/opt/kumomta/etc/policy/custom-shaping.toml',
  },
}
```

This section enables communication with the TSA daemon. The publish and subscribe URLs correspond to the TSA daemon's HTTP listener endpoint defined in its tsa_init.lua.  For a single node deployment the values shown here are sufficient.  You may list multiple publish and/or subscribe endpoints to publish to multiple hosts and read shaping configuration from multiple hosts, respectively. In addition, while the `setup_with_automation` call is aware of the community shaping rules file, any custom file must be identified in the `extra_files` directive as seen in the example above.

!!! warning
    As mentioned previously, your rules merge with the other files listed unless a given block has `replace_base=true`. To fully remove the defaults provided by the KumoMTA team you need the following:

    ```lua
    local shaping = require 'policy-extras.shaping'

    local shaper = shaping:setup_with_automation {
      publish = { 'http://127.0.0.1:8008' },
      subscribe = { 'http://127.0.0.1:8008' },
      no_default_files=true,
      extra_files = { 
            '/opt/kumomta/share/community/shaping.toml', 
            '/opt/kumomta/etc/policy/custom-shaping.toml',
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

## Configure The `tsa_init.lua` File

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

!!! note
    The `tsa_init.lua` has no implicit loading of the default `shaping.toml` file. To avoid loading the default file simply omit it.

## Writing Automation Rules to `shaping.toml`

Automation rules are written to the same shaping configuration files used for regular traffic shaping, under the same domain or provider scopes. Each rule defines a regex to be matched, an action to be taken in the event of a match, the trigger conditions for triggering the regex match, and the duration that the action should be applied for.

{% call toml_data() %}

[["gmail.com".automation]]
regex = "This message does not have authentication information"
action = "SuspendTenant"
duration = "3 hours"

[[provider."yahoo".automation]]
regex = "\\[TS04\\]"
action = "Suspend"
duration = "2 hours"

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

The full set of Traffic Shaping Automation actions is available on the [traffic shaping](../../reference/kumo.shaping/load.md#traffic-shaping-automation-rules) page of the Reference Manual.

## Monitoring the TSA Daemon

Adjustments to the traffic shaping rules are achieved by creating a custom `shaping.toml` file that is maintained by the TSA daemon and loaded as an overlay on the existing `shaping.toml file created by the user.

The generated TOML can be monitored by making an HTTP request. One example using curl:

```console
$ curl -s 'http://localhost:8008/get_config_v1/shaping.toml'
# Generated by tsa-daemon
# Number of entries: 0
```

This call returns the current set of shaping rules in the same format as shaping.toml, the example is of an empty set.

## Debugging

If the tsa-daemon does not appear to be working, you can check to see if it is running with `sudo systemctl status kumo-tsa-daemon` which should return a message that includes "active (running)".  If not you can stop and start it in a similar way.

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

Data being sent to the TSA daemon is handled the same as any other message in KumoMTA and will follow the same retry rules. The default is to retry in 20 minutes with exponential backoff.  If desired, this (or any other) scheduled queue can be customized with the [get_queue_config](https://docs.kumomta.com/reference/events/get_queue_config/) hook or in your shaping.toml file.

## Clustering

There are special considerations when implementing traffic shaping in a clustered environment, see the [Clustering Chapter](../clustering/index.md) for more information.
