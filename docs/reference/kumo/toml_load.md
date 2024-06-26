# `kumo.toml_load(FILENAME)`

!!! warning
    This function has moved to the [kumo.serde](../kumo.serde/index.md) module and
    will be removed in a future release.
    {{since('dev', inline=True)}}

Reads the content of the file name `FILENAME` and parses it as TOML,
returning a lua representation of the parsed TOML.

See also [kumo.toml_parse](toml_parse.md), [kumo.toml_encode](toml_encode.md)
and [kumo.toml_encode_pretty](toml_encode_pretty.md)
