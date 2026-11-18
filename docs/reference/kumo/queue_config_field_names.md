# kumo.queue_config_field_names

```lua
local names = kumo.queue_config_field_names()
```

{{since('dev')}}

Returns the names of the options accepted by
[kumo.make_queue_config](make_queue_config/index.md), as an array of strings.

The list is the field list `kumo.make_queue_config` validates against.

This is intended for policy helpers that read configuration files whose keys
mix queue options with other names, and need to tell them apart before
calling `kumo.make_queue_config`. The `policy-extras.queue` policy module
uses it this way when parsing `queues.toml`.

```lua
local is_option = {}
for _, name in ipairs(kumo.queue_config_field_names()) do
  is_option[name] = true
end

assert(is_option.max_age)
assert(not is_option.some_tenant_name)
```

See also:

  * [kumo.egress_source_field_names](egress_source_field_names.md)
  * [kumo.listener_domain_field_names](listener_domain_field_names.md)
