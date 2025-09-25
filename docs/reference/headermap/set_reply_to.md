# set_reply_to

```lua
headers:set_reply_to(TO)
```

{{since('dev')}}

Assign `VALUE` to the `Reply-To` header.

`VALUE` may be either a string or an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
