# additional_message_rate_throttles

{{since('2024.11.08-d383b033')}}

Specifies additional message rate constraints that cut across the
per-site-per-source scoping of the [max_message_rate](max_message_rate.md)
option.

The value is a map from the *limit name* to the desired throttle spec.

For example, you could implement a global outbound message rate of `4000/s`
like this:

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    additional_message_rate_throttles = {
      ['global-message-rate'] = '4000/s',
    },
  }
end)
```

or you could set up a source-specific limit that is shared by all
domains with a particular suffix match something like this, including both
the source and your candidate domain name in the *limit name*:

```lua
local utils = require 'policy-extras.policy_utils'

local RATES = {
  ['.outlook.com'] = '100/s',
  ['.example.com'] = '10/s',
}

kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  local rates = {}
  for suffix, value in pairs(RATES) do
    if utils.ends_with(domain, suffix) then
      limits[string.format('site-rate-limit-for-%s-%s', suffix, source)] =
        value
    end
  end
  return kumo.make_egress_path {
    additional_message_rate_throttles = rates,
  }
end)
```

You can mix all of the above with the built-in `max_message_rate`:

```lua
local utils = require 'policy-extras.policy_utils'

local RATES = {
  ['.outlook.com'] = '100/s',
  ['.example.com'] = '10/s',
}

kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  local rates = {
    -- No more than 4000/s connections globally
    ['global-message-rate'] = '4000/s',
  }
  -- Apply domain+source specific limits
  for suffix, value in pairs(RATES) do
    if utils.ends_with(domain, suffix) then
      limits[string.format('site-rate-limit-for-%s-%s', suffix, source)] =
        value
    end
  end
  return kumo.make_egress_path {
    additional_message_rate_throttles = rates,
    -- no more than 200/s from a given source to this specific site
    max_message_rate = '200/s',
  }
end)
```

When a message is eligible to be delivered from the ready queue, the system
will sort the overall set of rate limits, including the `max_message_rate`
option, from smallest to highest, then check and increment each in that order.
This minimizes the chances that we'll redundantly consume an available slot
from the larger allocation only to trip over one of the smaller ones.

!!! note
    When choosing a name for your throttle, you can select any name you like,
    but you should avoid using the prefix `kumomta.` as that is used by kumomta
    and you do not want to collide with its own limit names.

See also [max_message_rate](max_message_rate.md).

