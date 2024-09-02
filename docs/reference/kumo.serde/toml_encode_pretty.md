# `kumo.serde.toml_encode_pretty(VALUE)`

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.toml_encode_pretty`.

Attempts to encode VALUE as its TOML equivalent, and returns that value
serialized as a string, using pretty, more human readable representation.

This is logically the opposite of [kumo.serde.toml_parse](toml_parse.md).

See also [kumo.serde.toml_load](toml_load.md),
[kumo.serde.toml_parse](toml_parse.md) and
[kumo.serde.toml_encode](toml_encode.md)


