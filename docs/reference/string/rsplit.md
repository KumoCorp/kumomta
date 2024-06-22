# `kumo.string.rsplit(STRING, PATTERN)`

{{since('dev')}}

Splits `STRING` by `PATTERN` in reverse order, returning an array-style table
holding the substrings.

```lua
assert(
  kumo.json_encode(kumo.string.rsplit('Mary had a little lamb', ' '))
    == '["lamb","little","a","had","Mary"]'
)
```


