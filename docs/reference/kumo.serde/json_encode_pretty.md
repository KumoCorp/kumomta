# `kumo.serde.json_encode_pretty(VALUE)`

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.json_encode_pretty`.


Attempts to encode VALUE as its JSON equivalent, and returns that value
serialized as a string, using pretty, more human readable representation.

This is logically the opposite of [kumo.serde.json_parse](json_parse.md).

See also [kumo.serde.json_load](json_load.md), [kumo.serde.json_parse](json_parse.md)
and [kumo.serde.json_encode](json_encode.md)


