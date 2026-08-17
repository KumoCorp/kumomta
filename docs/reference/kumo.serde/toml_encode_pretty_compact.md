# kumo.serde.toml_encode_pretty_compact

```lua
kumo.serde.toml_encode_pretty_compact(VALUE)
```

{{since('dev')}}

Attempts to encode VALUE as its TOML equivalent and returns that
value serialized as a string, with two layout normalizations applied
on top of the default pretty representation:

  * Keys are sorted alphabetically at every nesting level so the
    output is stable and scan-friendly regardless of the order in
    which the underlying table was constructed.
  * Empty tables are emitted inline as `key = {}` rather than as a
    standalone `[key]` section header.

See also [kumo.serde.toml_encode](toml_encode.md),
[kumo.serde.toml_encode_pretty](toml_encode_pretty.md),
[kumo.serde.toml_parse](toml_parse.md) and
[kumo.serde.toml_load](toml_load.md).
