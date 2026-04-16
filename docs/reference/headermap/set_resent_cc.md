# set_resent_cc

```lua
headers:set_resent_cc(TO)
```

{{since('2025.10.06-5ec871ab')}}

Assign `VALUE` to the `Resent-Cc` header.

`VALUE` may be either a string or an [AddressList](index.md#addresslist).

If you assign using a string, the string will be parsed and validated as being
compatible with [AddressList](index.md#addresslist) before allowing the assigment to proceed.
