# set_resent_to

```lua
headers:set_resent_to(VALUE)
```

{{since('dev')}}

Assign `VALUE` to the `Resent-To` header.

`VALUE` may be either a string or an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
