# `kumo.string.split(STRING, PATTERN)`

{{since('2024.09.02-c5476b89')}}

Splits `STRING` by `PATTERN`, returning an array-style table
holding the substrings.

```lua
assert(
  kumo.json_encode(kumo.string.split('Mary had a little lamb', ' '))
    == '["Mary","had","a","little","lamb"]'
)
```



