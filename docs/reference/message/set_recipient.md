# `message:set_recipient(ENVELOPE)`

{{since('dev')}}

Sets the envelope recipient of the message.  The value can be an
[EnvelopeAddress](../address/index.md) or a string that can be
parsed into an `EnvelopeAddress`.

```lua
message:set_recipient 'someone.else@example.com'
```

See also [message:set_sender](set_sender.md).

