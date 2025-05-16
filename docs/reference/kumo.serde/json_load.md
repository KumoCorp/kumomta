# kumo.serde.json_load

```lua
kumo.serde.json_load(FILENAME)
```

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.json_load`.


Reads the content of the file name `FILENAME` and parses it as JSON,
returning a lua representation of the parsed JSON.

This json loading implementation will accept C style block comments, C++ style
single line comments and shell style single line comments.  Comments will be
treated as though they were spaces prior to being parsed by the underlying json
parser.

See also [kumo.serde.json_parse](json_parse.md), [kumo.serde.json_encode](json_encode.md)
and [kumo.serde.json_encode_pretty](json_encode_pretty.md)
