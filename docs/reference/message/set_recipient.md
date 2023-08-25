# `message:set_recipient(ENVELOPE)`

{{since('2023.08.22-4d895015')}}

Sets the envelope recipient of the message.  The value can be an
[EnvelopeAddress](../address/index.md) or a string that can be
parsed into an `EnvelopeAddress`.

```lua
message:set_recipient 'someone.else@example.com'
```

See also [message:set_sender](set_sender.md).

