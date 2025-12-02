# batch_handling

{{since('2025.12.02-67ee9e96')}}

SMTP messages can have an envelope that includes multiple recipients.  Each
recipient will receive a copy of the message.  If multiple recipients share the
same mailbox provider then it is advantageous from a bandwidth and efficiency
perspective to relay that message to that provider as a single message with a
list of multiple recipients, rather than sending one distinct copy per
recipient.

The `batch_handling` option specifies how incoming multi-recipient transactions
are split into outgoing batches.

It can have one of two values:

  * `"BifurcateAlways"` - this is the default and recommended setting for
    sender-focused deployments. Every incoming recipient is placed into a
    separate batch and tracked separately.
  * `"BatchByDomain"` - recipients with exactly the same domain portion are
    grouped together, resulting in one outgoing batch per unique domain.

If you have more advanced requirements around managing batching/splitting, then
you can implement them via the
[smtp_server_split_transaction](../../events/smtp_server_split_transaction.md)
event handler.

```lua
kumo.start_esmtp_listener {
  batch_handling = 'BatchByDomain',
}
```


