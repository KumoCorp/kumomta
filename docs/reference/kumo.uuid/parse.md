# `kumo.uuid.parse(UUID)`

{{since('2024.09.02-c5476b89')}}

Parses a UUID from a string and into a UUID object.

```lua
local u = kumo.uuid.parse '{69994630-3e27-11ef-91fc-cc28aa0a5c5a}'
assert(u.hyphenated == '69994630-3e27-11ef-91fc-cc28aa0a5c5a')
```

See [The UUID Object](index.md#the-uuid-object) for more information about the
returned UUID object.
