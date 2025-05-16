---
status: deprecated
---

# kumo.json_parse

```lua
kumo.json_parse(STRING)
```

!!! warning
    This function has moved to the [kumo.serde](../kumo.serde/index.md) module and
    will be removed in a future release.
    {{since('2024.09.02-c5476b89', inline=True)}}

Parses STRING as JSON, returning a lua representation of the parsed JSON.

This json parsing implementation will accept C style block comments, C++ style
single line comments and shell style single line comments.  Comments will be
treated as though they were spaces prior to being parsed by the underlying json
parser.

This is logically the opposite of [kumo.json_encode](json_encode.md).

See also [kumo.json_load](json_load.md), [kumo.json_encode](json_encode.md)
and [kumo.json_encode_pretty](json_encode_pretty.md)
