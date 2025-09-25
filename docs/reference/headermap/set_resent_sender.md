# set_resent_sender

```lua
headers:set_resent_sender(VALUE)
```

{{since('dev')}}

Assign `VALUE` to the `Resent-Sender` header.

`VALUE` may be either a string or a [Mailbox](index.md#mailbox).

If you assign using a string, the string will be parsed and validated as being
compatible with [Mailbox](index.md#mailbox) before allowing the assigment to proceed.
