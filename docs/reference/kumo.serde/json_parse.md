# `kumo.serde.json_parse(STRING)`

{{since('dev')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.json_parse`.


Parses STRING as JSON, returning a lua representation of the parsed JSON.

This json parsing implementation will accept C style block comments, C++ style
single line comments and shell style single line comments.  Comments will be
treated as though they were spaces prior to being parsed by the underlying json
parser.

This is logically the opposite of [kumo.serde.json_encode](json_encode.md).

See also [kumo.serde.json_load](json_load.md),
[kumo.serde.json_encode](json_encode.md) and
[kumo.serde.json_encode_pretty](json_encode_pretty.md)
