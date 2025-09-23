# recipient

```lua
message:recipient()
```

Returns the envelope recipient of the message.  The return value is an
[EnvelopeAddress](../address/index.md)

See also [message:sender](sender.md).

## Recipient List

{{since('dev')}}

If the message is part of an SMTP batch with more than a single recipient then
this method can return an array style table holding one
[EnvelopeAddress](../address/index.md) for each recipient.

If you'd rather always deal with a list of recipients, even if there is
just a single recipient, then you can use [message:recipient_list](recipient_list.md).
