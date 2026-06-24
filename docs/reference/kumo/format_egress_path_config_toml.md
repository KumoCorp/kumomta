# kumo.format_egress_path_config_toml

```lua
local text = kumo.format_egress_path_config_toml(path_config)
```

{{since('dev')}}

Given an egress path configuration table (as returned by
[kumo.invoke_get_egress_path_config](invoke_get_egress_path_config.md)
or constructed via
[kumo.make_egress_path](make_egress_path/index.md)), returns the
configuration serialized as compact pretty TOML.

Equivalent to passing the same value to
[kumo.serde.toml_encode_pretty_compact](../kumo.serde/toml_encode_pretty_compact.md),
except that the typed Rust representation of the egress path is used
during serialization. This preserves the distinction between
array-valued and map-valued fields, which is lost when the generic
helper materializes from a Lua table.

The output round-trips back into an `EgressPathConfig` via
[kumo.serde.toml_parse](../kumo.serde/toml_parse.md).
