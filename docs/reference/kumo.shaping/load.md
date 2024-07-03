# `kumo.shaping.load({PATHS})`

{{since('2023.08.22-4d895015')}}

This function will load traffic shaping data from the list of filenames or HTTP/HTTPS URLs
that is passed to it.

For example, in the `tsa-daemon` configuration, you might use it like this:

```lua
kumo.on('tsa_load_shaping_data', function()
  local shaping = cached_load_shaping_data {
    -- This is the default file used by the shaping helper
    -- in KumoMTA, which references the community shaping rules
    '/opt/kumomta/share/policy-extras/shaping.toml',

    -- and maybe you have your own rules
    '/opt/kumomta/policy/shaping.toml',
  }
  return shaping
end)
```

## Shaping Data Format

If a given path ends with `.toml`, it will be interpreted as TOML. Otherwise, it will
be interpreted as JSON.

This documentation uses TOML as it is a bit more friendly for humans to read and write.

Shaping data is considered as an ordered series of shaping configuration files,
where successive files layer and merge over earlier files.

### Domains and Merging

Each file contains information keyed by the destination domain name.

The values in a domain section must be valid values for
[kumo.make_egress_path](../kumo/make_egress_path.md), with a couple of special
additional values indicated below.

The special domain name `"default"` is used to define common, shared values,
used as the basis for every domain.

In this example, `connection_limit` and `enable_tls` are set for every domain.
However, when sending to `gmail.com`, its `connection_limit` of `100` will
override the `10` from the `default` section, and it will also use the
`enable_tls` value from the default section; the values are merged together:

{% call toml_data() %}
["default"]
connection_limit = 10
enable_tls = "Opportunistic"

["gmail.com"]
connection_limit = 100
{% endcall %}

Specifying the same domain in separate files will also merge the configuration,
which allows us to share community-provided base rules that you can then choose
to override without replacing everything for that domain.

However, if you want to completely replace the information for a domain, you
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

### MX Rollup

By default, the shaping rules associated with a domain are applied to the
*site_name* that is constructed from the list of MX hosts in DNS for that
domain.  That allows the rules to apply to every domain that uses a shared set
of MXs, for example, every G-Suite hosted domain will share `gmail.com` rules.

For some domains this may not be desirable; in this situations you can prevent
the rule from using the *site_name* by setting `mx_rollup = false`:

{% call toml_data() %}
["example.com"]
mx_rollup = false
{% endcall %}

### Per-Source Rules

You can provide a list of source-specific rules like this:

{% call toml_data() %}
["example.com".sources."my source name"]
connection_limit = 5
{% endcall %}

The section is named in the format `DOMAIN.sources.SOURCE`.  Both the `DOMAIN`
and the `SOURCE` must be quoted using double-quotes in order for the structure
to be correct.

### Traffic Shaping Automation Rules

The shaping data can include automation rules that will be evaluated by the
optional `tsa-daemon` process.

Here's an example that defines policy for `yahoo.com`:

{% call toml_data() %}
["yahoo.com"]
max_deliveries_per_connection = 20

[["yahoo.com".automation]]
regex = "\\[TS04\\]"
action = "Suspend"
duration = "2 hours"
{% endcall %}

In TOML, the `[[DOMAIN.automation]]` syntax appends an additional entry to the
list of `automation` rules in `DOMAIN`.

This particular rule uses a regex to look for `[TS04]` in the delivery status
responses from yahoo, and when it matches, the action taken is to suspend
delivery to yahoo.com from the triggering source.  Other sources will be
handled independently.

The following fields are possible in an automation rule:

 * `regex` - required string, the regular expression used to match the rule.
   [Supported Regex Syntax is documented here](https://docs.rs/fancy-regex/latest/fancy_regex/#syntax)
 * `action` - required action to take.  Can be one of:
    * `"Suspend"` - Suspend delivery
    * `{SetConfig{name="NAME", value="VALUE"}}` - define a configuration override that sets `NAME=VALUE`.
 * `trigger` - optional. Can be one of:
    * `"Immediate"` - this is the default. The action is taken each time a `regex` matches.
    * `{Threshold="10/hr"}` - defines a threshold; the action won't trigger in this case until 10 events have occurred in the preceding hour.
 * `duration` - required string specifying the duration of the effects of the action.

{{since('2024.06.10-84e84b89')}}

The following new actions are now supported:

 * `"SuspendTenant"` - Generate a suspension for all scheduled queues that have
   both the *tenant* and the destination domain of the triggering record. If no
   tenant was assigned, this action has no effect.
 * `"SuspendCampaign"` - Generate a suspension for all scheduled queues that
   have both the *tenant*, *campaign* and the destination domain of the
   triggering record.  If no tenant was assigned, this action has no effect.
   If no campaign was assigned, behave as though `"SuspendTenant"` was the
   action.

