# `kumo.serde.toml_encode(VALUE)`

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.toml_encode`.

Attempts to encode VALUE as its TOML equivalent, and returns that value
serialized as a string.

This is logically the opposite of [kumo.serde.toml_parse](toml_parse.md).

See also [kumo.serde.toml_load](toml_load.md),
[kumo.serde.toml_parse](toml_parse.md) and
[kumo.serde.toml_encode_pretty](toml_encode_pretty.md)


