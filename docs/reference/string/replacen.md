# `kumo.string.replacen(STRING, FROM, TO, COUNT)`

{{since('2024.09.02-c5476b89')}}

Replaces the first N matches of `FROM` with `TO` and returns the resulting string.

```lua
assert(
  kumo.string.replacen('foo foo 123 foo', 'foo', 'new', 2)
    == 'new new 123 foo'
)
```

