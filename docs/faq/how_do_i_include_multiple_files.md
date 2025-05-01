# How do I include multiple configuration files from a directory?

If you are looking to structure your configuration as a collection of include files in a directory hierarchy you can use the [kumo.glob](../reference/kumo/glob.md) function to build out the appropriate include string.

For example, the following code will load the shaping helper with all toml files in a given directory:

```lua
local shaper = shaping:setup_with_automation {
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
  extra_files = {'/opt/kumomta/etc/policy/shaping.toml',
                 '/opt/kumomta/etc/policy/vmta_shaping.toml',
                 '/opt/kumomta/etc/policy/automation_rules.toml',
                 table.unpack(kumo.glob '/opt/kumomta/etc/shaping/*.toml'),
         },
}
```
