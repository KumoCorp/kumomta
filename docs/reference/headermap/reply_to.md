# reply_to

```lua
local to = headers:reply_to()
```

{{since('2025.10.06-5ec871ab')}}

Parses the `Reply-To` header and returns it in [AddressList](index.md#addresslist) representation.
