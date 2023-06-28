# `kumo.on('smtp_server_mail_from', function(sender, conn_meta))`

Called by the ESMTP server in response to the client issuing a `"MAIL FROM"`
command.  The event handler is passed the *sender address* parameter from
the `MAIL FROM` command.

The *sender address* is an [EnvelopeAddress](../address/index.md) object.

{{since('dev', indent=True)}}
    The *conn_meta* parameter represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

You may choose to reject the connection via [kumo.reject](../kumo/reject.md).

```lua
kumo.on('smtp_server_mail_from', function(sender)
  if sender.domain == 'bad.domain' then
    kumo.reject(420, 'not thanks')
  end
end)
```
