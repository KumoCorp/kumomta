# `kumo.serde.toml_load(FILENAME)`

{{since('dev')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.toml_load`.

Reads the content of the file name `FILENAME` and parses it as TOML,
returning a lua representation of the parsed TOML.

See also [kumo.serde.toml_parse](toml_parse.md),
[kumo.serde.toml_encode](toml_encode.md) and
[kumo.serde.toml_encode_pretty](toml_encode_pretty.md)
