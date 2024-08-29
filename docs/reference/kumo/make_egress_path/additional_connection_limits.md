# additional_connection_limits

{{since('dev')}}

Specifies additional connection limit constraints that cut across the
per-site-per-source scoping of the [connection_limit](connection_limit.md)
option.

The value is a map from the *limit name* to the desired limit.

For example, you could implement a global outbound connection limit of 100
connections like this:

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    additional_connection_limits = {
      ['global-connection-limit'] = 100,
    },
  }
end)
```

or you could set up a source-specific connection limit that is shared by all
domains with a particular suffix match something like this, including both
the source and your candidate domain name in the *limit name*:

```lua
local utils = require 'policy-extras.policy_utils'

local LIMITS = {
  ['.outlook.com'] = 100,
  ['.example.com'] = 32,
}

kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  local limits = {}
  for suffix, value in pairs(LIMITS) do
    if utils.ends_with(domain, suffix) then
      limits[string.format('site-limit-for-%s-%s', suffix, source)] = value
    end
  end
  return kumo.make_egress_path {
    additional_connection_limits = limits,
  }
end)
```

You can mix all of the above with the built-in `connection_limit`:

```lua
local utils = require 'policy-extras.policy_utils'

local LIMITS = {
  ['.outlook.com'] = 100,
  ['.example.com'] = 32,
}

kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  local limits = {
    -- No more than 100 connections globally
    ['global-connection-limit'] = 100,
  }
  -- Apply domain+source specific limits
  for suffix, value in pairs(LIMITS) do
    if utils.ends_with(domain, suffix) then
      limits[string.format('site-limit-for-%s-%s', suffix, source)] = value
    end
  end
  return kumo.make_egress_path {
    -- no more than 10 connections from a given source to this specific site
    connection_limit = 10,
    additional_connection_limits = limits,
  }
end)
```

When a connection is eligible to be established, the system will sort the
overall set of connection limits, including the `connection_limit` limit
option, from smallest to highest, then acquire a lease to connect in that
order.  This minimizes the chances that we'll redundantly consume an available
slot from the larger allocation only to trip over one of the smaller ones.

!!! note
    When choosing a name for your limit, you can select any name you like,
    but you should avoid using the prefix `kumomta.` as that is used by
    kumomta and you do not want to collide with its own limit names.

See also [connection_limit](connection_limit.md).
