# `kumo.string.split_ascii_whitespace(STRING)`

{{since('2024.09.02-c5476b89')}}

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
