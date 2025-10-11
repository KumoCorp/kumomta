# smtp_server_split_transaction

{{since('dev')}}

```lua
kumo.on('smtp_server_split_transaction', function(message, conn_meta) end)
```

Called by the ESMTP server to decide whether and how to split the recipient
list into groups of recipients at the same destination site.

SMTP messages can have an envelope that includes multiple recipients.  Each
recipient will receive a copy of the message.  If multiple recipients share the
same mailbox provider then it is advantageous from a bandwidth and efficiency
perspective to relay that message to that provider as a single message with a
list of multiple recipients, rather than sending one distinct copy per
recipient.

This event gives you control over how the incoming transaction is split and the
recipients are grouped together.

You do not need to implement this event handler in the vast majority of cases!
Look at
[start_esmtp_listener.batch_handling](../kumo/start_esmtp_listener/batch_handling.md)
for a much simpler way to express the most common choices.

You can use [message:recipient_list](../message/recipient_list.md) to retrieve
the recipient list from the message.  That might give you something like:

```
{ 'fred@gmail.com', 'pete@gmail.com', 'joe@hotmail.com' }
```

(but each element is an [EnvelopeAddress](../address/index.md) rather than a string).

The return value must be an array of arrays of addresses.  For example, if you
wish to recreate the `batch_handling = "BifurcateAlways"` mode of operation,
the shape of the result will look like this:

```
{
    { 'fred@gmail.com' },
    { 'pete@gmail.com' },
    { 'joe@hotmail.com' }
}
```

If you wish to recreate the `batch_handling = "BatchByDomain"` mode of operation,
it will instead look like this, with all of the gmail addresses in the same
top level array:

```
{
    { 'fred@gmail.com', 'pete@gmail.com' },
    { 'joe@hotmail.com' }
}
```

Returning `nil` (or not explicitly returning anything) from
`smtp_server_split_transaction` will cause the `batch_handling` option to be
consulted to decide how splitting/batching will occur.

When a multi-recipient message is routed onwards, the domain of the first
recipient is used to resolve the MX and decide which host to connect to.  In
the example above, `fred@gmail.com` will be used to resolve routing for the
first batch, while `joe@hotmail.com` will be used to resolve routing for the
second batch.

!!! warning
    While you can create arbitrary batches with this event handler, if you
    create non-sensical batches you should expect for messages to fail to
    deliver.  For example, if you group `user@gmail.com` together with
    `other.user@hotmail.com` then when the message is attempted, gmail (from
    the first recipient) will likely reject the hotmail address because gmail
    is not responsible for and may choose not to relay the hotmail recipient.

!!! note
    This event is called even if the recipient list has a single entry

It is technically possible to synthesize additional recipients by including
them in the returned list of batches, but it is recommended that you perform
recipient list modification in the [smtp_server_data](smtp_server_data.md)
event rather than this event.

## Example: equivalent to batch_handling=BifurcateAlways

Every incoming recipient is placed into a separate batch and tracked separately.

```lua
kumo.on('smtp_server_split_transaction', function(message, conn_meta)
  local split = {}
  for _, recip in ipairs(message:recipient_list()) do
    table.insert(split, { recip })
  end
  return split
end)
```

## Example: equivalent to batch_handling=BatchByDomain

Recipients with exactly the same domain portion are grouped together.

```lua
kumo.on('smtp_server_split_transaction', function(message, conn_meta)
  local by_domain = {}
  for _, recip in ipairs(message:recipient_list()) do
    local domain = recip.domain:lower()
    if not by_domain[domain] then
      by_domain[domain] = {}
    end
    table.insert(by_domain[domain], recip)
  end
  return by_domain
end)
```
