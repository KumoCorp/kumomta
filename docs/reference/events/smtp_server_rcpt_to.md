# `kumo.on('smtp_server_rcpt_to', function(recipient))`

Called by the ESMTP server in response to the client issuing a `"RCPT TO"`
command.  The event handler is passed the *recipient address* parameter from
the `RCPT TO` command.

The *recipient address* is an [EnvelopeAddress](../address/index.md) object.

You may choose to reject the connection via [kumo.reject](../kumo/reject.md).

```lua
kumo.on('smtp_server_rcpt_to', function(recipient)
  if recipient.domain == 'bad.domain' then
    kumo.reject(420, 'not thanks')
  end
end)
```

