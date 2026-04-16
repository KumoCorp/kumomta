# bcc

```lua
local bcc = headers:bcc()
```

{{since('2025.10.06-5ec871ab')}}

Parses the `Bcc` header from the headermap and returns the corresponding
[AddressList](index.md#addresslist) representation of the header.

