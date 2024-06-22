# `kumo.string.rsplitn(STRING, LIMIT, PATTERN)`

{{since('dev')}}

Splits `STRING` by `PATTERN` in reverse order, returning an array-style table
holding at most `LIMIT` substrings.

```lua
assert(
  kumo.json_encode(kumo.string.rsplitn('Mary had a little lamb', 3, ' '))
    == '["lamb","little","Mary had a"]'
)
```



