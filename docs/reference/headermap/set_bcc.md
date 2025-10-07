# set_bcc

```lua
headers:set_bcc(VALUE)
```

{{since('2025.10.06-5ec871ab')}}

Assign the `VALUE` to the `Bcc` header.

`VALUE` may be either a `string` or be an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.

