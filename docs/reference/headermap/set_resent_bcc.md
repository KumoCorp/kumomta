# set_resent_bcc

```lua
headers:set_resent_bcc(TO)
```

{{since('2025.10.06-5ec871ab')}}

Assign `VALUE` to the `Resent-Bcc` header.

`VALUE` may be either a string or an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
