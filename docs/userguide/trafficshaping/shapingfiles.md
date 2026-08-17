---
description: Configure KumoMTA traffic shaping with TOML or JSON files, covering the default and community shaping.toml, custom files, and configuration precedence.
---

# Traffic Shaping Configuration Files

The `shaping.lua` policy reads one or more configuration files in either TOML or JSON formats.

## The Default shaping.toml File

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
enable_tls = 'Required'
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

As a special case, the domain can be named `default`, in which case those settings will apply globally. The global settings are superseded by the domain settings, which are superseded by the source settings.

The full set of available options is listed in the [kumo.make_egress_path](../../reference/kumo/make_egress_path/index.md) page of the Reference Manual.

The full set of Traffic Shaping Automation actions is available on the [traffic shaping](../../reference/kumo.shaping/load.md) page of the Reference Manual.

## The Community shaping.toml File

In addition, the KumoMTA Github repo includes a traffic shaping rules file maintained by the community, available at [https://github.com/KumoCorp/kumomta/blob/main/assets/community/shaping.toml](https://github.com/KumoCorp/kumomta/blob/main/assets/community/shaping.toml) that can also be loaded explicitly as an additional resource for users.

## Custom Shaping Files

Finally, users can opt to create their own shaping rules file that can augment or replace the rules defined in the previous two files.

If you intend to manually maintain your own shaping rules, we recommend using TOML as your file format, whereas if you intend to automatically maintain your traffic shaping rules, we recommend using JSON as your file format. An example in TOML format is shown above.

While you can place a custom shaping file at any readable location, it is common to place the file at `/opt/kumomta/etc/policy/custom-shaping.[toml|json]` for consistency with examples used elsewhere in the documentation.

## Order of Definition and Configuration File Precedence

The order in which your traffic shaping configuration files are defined affects how they are loaded and in turn which options are preserved when there is a conflict between the files.

The `shaping.lua` helper reads files sequentially, and the most recently defined file will overwrite any option set by a previously defined file.

Because of this, we recommend setting up shaping following this pattern:

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

!!! note
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
