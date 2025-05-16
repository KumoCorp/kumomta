---
status: deprecated
---

# kumo.toml_parse

```lua
kumo.toml_parse(STRING)
```

!!! warning
    This function has moved to the [kumo.serde](../kumo.serde/index.md) module and
    will be removed in a future release.
    {{since('2024.09.02-c5476b89', inline=True)}}

Parses STRING as TOML, returning a lua representation of the parsed TOML.

This is logically the opposite of [kumo.toml_encode](toml_encode.md).

See also [kumo.toml_load](toml_load.md), [kumo.toml_encode](toml_encode.md)
and [kumo.toml_encode_pretty](toml_encode_pretty.md)
