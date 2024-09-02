# `kumo.string.splitn(STRING, LIMIT, PATTERN)`

{{since('2024.09.02-c5476b89')}}

Splits `STRING` by `PATTERN`, returning an array-style table
holding at most `LIMIT` substrings.

```lua
assert(
  kumo.json_encode(kumo.string.splitn('Mary had a little lamb', 3, ' '))
    == '["Mary","had","a little lamb"]'
)
```




