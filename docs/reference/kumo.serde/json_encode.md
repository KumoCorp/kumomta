# kumo.serde.json_encode

```lua
kumo.serde.json_encode(VALUE)
```

{{since('2024.09.02-c5476b89')}}

!!! note
    In earlier versions of kumomta, this function was available
    as `kumo.json_encode`.

Attempts to encode VALUE as its JSON equivalent, and returns that value
serialized as a string.

This is logically the opposite of [kumo.serde.json_parse](json_parse.md).

See also [kumo.serde.json_load](json_load.md),
[kumo.serde.json_parse](json_parse.md) and
[kumo.serde.json_encode_pretty](json_encode_pretty.md)

