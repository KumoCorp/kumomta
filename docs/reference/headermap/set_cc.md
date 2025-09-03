# set_cc

```lua
headers:set_cc(VALUE)
```

{{since('dev')}}

Assign the `VALUE` to the `Cc` header.

`VALUE` may be either a `string` or be an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
