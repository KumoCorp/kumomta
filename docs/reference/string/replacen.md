# `kumo.string.replacen(STRING, FROM, TO, COUNT)`

{{since('dev')}}

Replaces the first N matches of `FROM` with `TO` and returns the resulting string.

```lua
assert(
  kumo.string.replacen('foo foo 123 foo', 'foo', 'new', 2)
    == 'new new 123 foo'
)
```

