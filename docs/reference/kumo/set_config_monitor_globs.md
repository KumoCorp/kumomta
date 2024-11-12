# `kumo.set_config_monitor_globs(GLOBS)`

{{since('2024.11.08-d383b033')}}

*GLOBS* is an array-style table listing out the set of glob expressions which
should be monitored as part of the [Configuration
Monitoring](../configuration.md#configuration-monitoring) system.

The effective default is as though you had this code in your policy file:

```lua
kumo.on('init', function()
  kumo.set_config_monitor_globs {
    '/opt/kumomta/etc/**/*.{lua,json,toml,yaml}',
  }
end)
```

You can specify multiple globs as makes sense for your deployment.


