# smtp_server_message_received

```lua
kumo.on('smtp_server_message_received', function(message, conn_meta) end)
```

Called by the ESMTP server after receiving the message data, but before
responding to the client in the live SMTP session.

If the client issued multiple `"RCPT TO"` commands in the same transaction,
each one will result in a separate message being created, and this event
will be triggered for each of them.

The event handler will be passed a [Message](../message/index.md) object.
The Message will always have a `Received` header prepended that captures trace
information about the sender.

{{since('2023.08.22-4d895015', indent=True)}}
    The *conn_meta* parameter represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

This event is the best place to carry out a number of important policy decisions:

* DKIM signing via [message:dkim_sign](../message/dkim_sign.md).
* Assigning the `"campaign"`, `"tenant"` and/or `"queue"` meta values via [msg:set_meta](../message/set_meta.md)

```lua
-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```
