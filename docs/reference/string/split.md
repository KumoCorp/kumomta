# `kumo.string.split(STRING, PATTERN)`

{{since('dev')}}

Splits `STRING` by `PATTERN`, returning an array-style table
holding the substrings.

```lua
assert(
  kumo.json_encode(kumo.string.split('Mary had a little lamb', ' '))
    == '["Mary","had","a","little","lamb"]'
)
```



