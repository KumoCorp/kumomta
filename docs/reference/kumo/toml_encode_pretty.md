# `kumo.toml_encode_pretty(VALUE)`

!!! warning
    This function has moved to the [kumo.serde](../kumo.serde/index.md) module and
    will be removed in a future release.
    {{since('dev', inline=True)}}

Attempts to encode VALUE as its TOML equivalent, and returns that value
serialized as a string, using pretty, more human readable representation.

This is logically the opposite of [kumo.toml_parse](toml_parse.md).

See also [kumo.toml_load](toml_load.md), [kumo.toml_parse](toml_parse.md)
and [kumo.toml_encode](toml_encode.md)


