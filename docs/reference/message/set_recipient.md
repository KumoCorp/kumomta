# set_recipient

```lua
message:set_recipient(ENVELOPE)
```

{{since('2023.08.22-4d895015')}}

Sets the envelope recipient of the message.  The value can be an
[EnvelopeAddress](../address/index.md) or a string that can be
parsed into an `EnvelopeAddress`.

```lua
message:set_recipient 'someone.else@example.com'
```

See also [message:set_sender](set_sender.md).

## Recipient List

{{since('2025.12.02-67ee9e96')}}

The value can be an array style table holding one `EnvelopeAddress` for each
recipient that you wish to assign to the message.

!!! note
    At the time of writing, only the SMTP and maildir delivery protocols have
    support for multi-recipient messages.

