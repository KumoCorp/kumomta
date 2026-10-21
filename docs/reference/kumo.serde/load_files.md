# kumo.serde.load_files

```lua
kumo.serde.load_files({ FILENAME, ... })
```

{{since('dev')}}

Reads and parses a list of files in one call, returning their parsed
values in the same order as the input list. A file whose name ends in
`.toml` is parsed as TOML; any other file is parsed as JSON, with comments
permitted as in [kumo.serde.json_load](json_load.md). Use this instead of
making one [kumo.serde.toml_load](toml_load.md) or
[kumo.serde.json_load](json_load.md) call per file, such as when loading a
directory of many small configuration files.

If any file cannot be read or parsed, the call raises an error naming that
file and returns no values.

See also [kumo.serde.toml_load](toml_load.md) and
[kumo.serde.json_load](json_load.md)
