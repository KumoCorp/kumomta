# `kumo.string.split_ascii_whitespace(STRING)`

{{since('dev')}}

Splits `STRING` by ASCII whitespace,.

To split by unicode `White_Space` instead,
use [split_whitespace](split_whitespace.md).

```lua
assert(
  kumo.json_encode(
    kumo.string.split_ascii_whitespace 'Mary had a little lamb'
  ) == '["Mary","had","a","little","lamb"]'
)
```
