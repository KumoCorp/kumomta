# `kumo.string.split_whitespace(STRING)`

{{since('dev')}}

Splits `STRING` by whitespace, as defined by the Unicode Derived Core Property `White_Space`.
If you only want to split on ASCII whitespace, use [split_ascii_whitespace](split_ascii_whitespace.md) instead, as it is cheaper.

```lua
assert(
  kumo.json_encode(kumo.string.split_whitespace 'Mary had a little lamb')
    == '["Mary","had","a","little","lamb"]'
)
```




