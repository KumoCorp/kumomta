# `kumo.serde.toml_parse(STRING)`

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.toml_parse`.

Parses STRING as TOML, returning a lua representation of the parsed TOML.

This is logically the opposite of [kumo.serde.toml_encode](toml_encode.md).

See also [kumo.serde.toml_load](toml_load.md),
[kumo.serde.toml_encode](toml_encode.md) and
[kumo.serde.toml_encode_pretty](toml_encode_pretty.md)
