# `kumo.on('smtp_server_message_received', function(message))`

Called by the ESMTP server after receiving the message data.

If the client issued multiple `"RCPT TO"` commands in the same transaction,
each one will result in a separate message being created, and this event
will be triggered for each of them.

The event handler will be passed a [Message](../message/index.md) object.
The Message will always have a `Received` header prepended that captures trace
information about the sender.

This event is the best place to carry out a number of important policy decisions:

* DKIM signing via [message:dkim_sign](../message/dkim_sign.md).
* Assigning the `"campaign"`, `"tenant"` and/or `"queue"` meta values via [msg:set_meta](../message/set_meta.md)

```lua
-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:sender().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    file_name = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```
