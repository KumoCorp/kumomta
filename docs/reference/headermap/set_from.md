# set_from

```lua
headers:set_from(VALUE)
```

{{since('dev')}}

Assign `VALUE` to the `From` header.

`VALUE` may be either a string or a [MailboxList](index.md#mailboxlist).

If you assign using a string, the string will be parsed and validated as being
compatible with [MailboxList](index.md#mailboxlist) before allowing the assigment to proceed.
