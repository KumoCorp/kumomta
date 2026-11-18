# kumo.listener_domain_field_names

```lua
local names = kumo.listener_domain_field_names()
```

{{since('dev')}}

Returns the names of the options accepted by
[kumo.make_listener_domain](make_listener_domain/index.md), as an array of
strings.

The list is read directly from the listener domain type in `kumod`, so these
are exactly the option names that `kumo.make_listener_domain` accepts.

Use this to classify a configuration key as a listener domain option before
deciding how to handle it, without calling `kumo.make_listener_domain` and
inspecting the result. The `policy-extras.listener_domains` helper uses it
this way to tell a listener domain option apart from other keys while
validating `listener_domains.toml`.

See also:

  * [kumo.queue_config_field_names](queue_config_field_names.md)
  * [kumo.egress_source_field_names](egress_source_field_names.md)
