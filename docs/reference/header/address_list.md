# address_list

```lua
local address_list = header.address_list
```

{{since('dev')}}

Reading the `address_list` field will attempt to interpret the contents of the
header as an [AddressList](../headermap/index.md#addresslist).

If the header value is not compatible with this representation, a lua error
will be raised.
