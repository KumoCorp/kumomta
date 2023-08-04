# `kumo.on('tsa_load_shaping_data', FUNCTION)`

{{since('dev')}}

Called by the `tsa-daemon` whenever it is going to evaluate a newly received log record.

The event must return a `Shaping` object, as can be obtained via
[kumo.shaping.load](../kumo.shaping/load.md).

It is recommended that you use the same list of filenames that you would use
with the shaping helper so that the two services have a consensus on the
shaping configuration.

`tsa-daemon` is only really concerned with automation rules defined by the
shaping configuration.

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
    '/opt/kumomta/policy/shaping.toml',
  }
  return shaping
end)
```
