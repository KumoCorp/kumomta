# `kumo.shaping.load({PATHS}, {OPTIONS})`

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

## Options Parameter

{{since('2024.11.08-d383b033')}}

The options parameter allows for the following fields:

* `aliased_site` - specifies the severity of aliases between domain blocks.
  Domains that resolve to the same site name are likely undesirable as they
  can lead to logical conflicts in the resulting configuration.
  The default value for this is `"Ignore"`, but you can specify `"Warn"` or
  `"Error"`.

* `dns_fail` - specifies the severity of DNS resolution failures for a domain
  block.
  The default value for this is `"Ignore"`, but you can specify `"Warn"` or
  `"Error"`.

* `local_load` - specifies the severity of a failure to load a local shaping
  file.
  The default value for this is `"Error"`, but you can specify `"Warn"` or
  `"Ignore"`.

* `null_mx` - how to treat a domain block when the DNS indicates that it
  is a NULL MX that doesn't receive mail.
  The default value for this is `"Ignore"`, but you can specify `"Warn"` or
  `"Error"`.

* `provider_overlap` - how to treat overlap between domain blocks and provider
  blocks. These are likely undesirable as they can lead to logical conflicts
  in the resulting configuration.
  The default value for this is `"Ignore"`, but you can specify `"Warn"` or
  `"Error"`.

* `remote_load` - specifies the severity of a failure to load a remote
  shaping file.
  The default value for this is `"Ignore"`, but you can specify `"Warn"` or
  `"Error"`.

* `skip_remote` - a boolean to indicate whether to skip loading remote shaping
  files.  The default is `false`, and the shaper will load remote shaping files.

* `http_timeout` - an optional duration string specifying the timeout to
  use for http requests made to fetch shaping data.  The default value if
  unspecified is `5s`. {{since('dev', inline=True)}}

## Shaping Data Format

If a given path ends with `.toml`, it will be interpreted as TOML. Otherwise, it will
be interpreted as JSON.

This documentation uses TOML as it is a bit more friendly for humans to read and write.

Shaping data is considered as an ordered series of shaping configuration files,
where successive files layer and merge over earlier files.

### Domains and Merging

Each file contains information keyed by the destination domain name.

The values in a domain section must be valid values for
[kumo.make_egress_path](../kumo/make_egress_path/index.md), with a couple of special
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
    * `{SetDomainConfig{name="NAME", value="VALUE"}}` - define a configuration
      override that sets `NAME=VALUE`, but with `mx_rollup=false`, even if the
      rule was defined inside a domain where `mx_rollup=true`. {{since('2024.11.08-d383b033',
      inline=True)}}
 * `trigger` - optional. Can be one of:
    * `"Immediate"` - this is the default. The action is taken each time a `regex` matches.
    * `{Threshold="10/hr"}` - defines a threshold; the action won't trigger in this case until 10 events have occurred in the preceding hour.
 * `duration` - required string specifying the duration of the effects of the action.
 * `match_internal` - optional boolean indicating whether internally generated
   response, that is, those that begin with the text `KumoMTA internal: `,
   should be allowed to match the rule. The default is `false`. Prior to the
   introduction of this option the behavior was equivalent to it being set to
   `true`. Unintentionally matching internal responses with a suspension rule
   could trigger surprising cyclical behavior where a suspension is triggered
   from a remote response and then subsequently the transient failures logged
   when messages hit that suspension would also match the rule and continue
   to apply and extend the lifetime of the suspension. {{since('2024.11.08-d383b033', inline=True)}}

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

{{since('2025.01.23-7273d2bc')}}

The following new actions are now supported:

 * `"Bounce"` - Generate a bounce for all scheduled queues that have the
   same destination domain as the triggered record.
 * `"BounceTenant"` - Generate a bounce for all scheduled queues that have
   both the same destination domain and *tenant* as the triggering record.
   If no tenant was assigned, this action has no effect.
 * `"BounceCampaign"` - Generate a bounce for all scheduled queues that have
   both the same destination domain, *tenant* AND *campaign* as the triggering
   record.  If no campaign was assigned, behave as though `"BounceTenant"` was
   the action.
