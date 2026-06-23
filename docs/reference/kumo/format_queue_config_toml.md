# kumo.format_queue_config_toml

```lua
local text = kumo.format_queue_config_toml(queue_config)
```

{{since('dev')}}

Given a scheduled-queue configuration table (as returned by
[kumo.invoke_get_queue_config](invoke_get_queue_config.md) or
constructed via [kumo.make_queue_config](make_queue_config/index.md)),
returns the configuration serialized as compact pretty TOML.

The typed Rust representation of the queue config is used during
serialization. This preserves the distinction between
array-valued and map-valued fields, which is lost when the generic
[kumo.serde.toml_encode_pretty_compact](../kumo.serde/toml_encode_pretty_compact.md)
materializes from a Lua table.

The output round-trips back into a `QueueConfig` via
[kumo.serde.toml_parse](../kumo.serde/toml_parse.md).
