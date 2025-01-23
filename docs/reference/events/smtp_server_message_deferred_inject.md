# `kumo.on('smtp_server_message_deferred_inject', function(message, conn_meta))`

{{since('2025.01.23-7273d2bc')}}

When [deferred_queue](../kumo/start_esmtp_listener/deferred_queue.md) is
enabled, this is called by after accepting the message data and responding to
the client.

If the client issued multiple `"RCPT TO"` commands in the same transaction,
each one will result in a separate message being created, and this event
will be triggered for each of them.

The event handler will be passed a [Message](../message/index.md) object.
The Message will always have a `Received` header prepended that captures trace
information about the sender.

The *conn_meta* parameter represents the connection metadata, but it is
synthesized from the metadata in the message.  It is provided as a
type-compatible way to reference the metadata for functions like SPF and DMARC
that accept connection metadata. While it is possible to update/set values in
this metadata object, they will not persist once the
`smtp_server_message_deferred_inject` event handler returns.  See [Connection
Metadata](../connectionmeta.md) for more information.

You can and should use this event to process things that you would normally
have done in the `smtp_server_message_received` event handler, such as:

* DKIM signing via [message:dkim_sign](../message/dkim_sign.md).
* Assigning the `"campaign"`, `"tenant"` and/or `"queue"` meta values via [msg:set_meta](../message/set_meta.md)

```lua
kumo.on('smtp_server_message_deferred_inject', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```
