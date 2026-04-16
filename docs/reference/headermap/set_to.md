# set_to

```lua
headers:set_to(VALUE)
```

{{since('2025.10.06-5ec871ab')}}

Assign `VALUE` to the `To` header.

`VALUE` may be either a string or an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
