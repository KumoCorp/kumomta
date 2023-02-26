# `kumo.on('smtp_server_mail_from', function(sender))`

Called by the ESMTP server in response to the client issuing a `"MAIL FROM"`
command.  The event handler is passed the *sender address* parameter from
the `MAIL FROM` command.

The *sender address* is an [EnvelopeAddress](../address/index.md) object.

You may choose to reject the connection via [kumo.reject](../kumo/reject.md).

```lua
kumo.on('smtp_server_mail_from', function(sender)
  if sender.domain == 'bad.domain' then
    kumo.reject(420, 'not thanks')
  end
end)
```
