# `kumo.json_encode_pretty(VALUE)`

!!! warning
    This function has moved to the [kumo.serde](../kumo.serde/index.md) module and
    will be removed in a future release.
    {{since('2024.09.02-c5476b89', inline=True)}}

Attempts to encode VALUE as its JSON equivalent, and returns that value
serialized as a string, using pretty, more human readable representation.

This is logically the opposite of [kumo.json_parse](json_parse.md).

See also [kumo.json_load](json_load.md), [kumo.json_parse](json_parse.md)
and [kumo.json_encode](json_encode.md)


