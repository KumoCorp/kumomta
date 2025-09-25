# set_resent_from

```lua
headers:set_resent_from(FROM)
```

{{since('dev')}}

Assign `VALUE` to the `Resent-From` header.

`VALUE` may be either a string or an [MailboxList](index.md#mailboxlist).

If you assign using a string, the string will be parsed and validated as being
compatible with [MailboxList](index.md#mailboxlist) before allowing the assigment to proceed.
