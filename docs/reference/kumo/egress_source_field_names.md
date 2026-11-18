# kumo.egress_source_field_names

```lua
local names = kumo.egress_source_field_names()
```

{{since('dev')}}

Returns the names of the options accepted by
[kumo.make_egress_source](make_egress_source/index.md), as an array of strings.

It is generated from the same Rust struct that `kumo.make_egress_source`
validates against, not maintained by hand.

This is intended for policy helpers that need to tell whether a key in a
configuration file is an egress source option before calling
`kumo.make_egress_source`. The `policy-extras.sources` helper uses it when
validating `sources.toml`.

See also:

  * [kumo.queue_config_field_names](queue_config_field_names.md)
  * [kumo.listener_domain_field_names](listener_domain_field_names.md)
