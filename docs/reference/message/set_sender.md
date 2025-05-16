# set_sender

```lua
message:set_sender(ENVELOPE)
```

{{since('2023.08.22-4d895015')}}

Sets the envelope sender of the message.  The value can be an
[EnvelopeAddress](../address/index.md) or a string that can be
parsed into an `EnvelopeAddress`.

```lua
message:set_sender(string.format('bounce-%s@%s', HASH, DOMAIN))
```

See also [message:set_recipient](set_recipient.md).

