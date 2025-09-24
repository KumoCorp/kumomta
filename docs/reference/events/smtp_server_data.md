# smtp_server_data

{{since('dev')}}

```lua
kumo.on('smtp_server_data', function(message, conn_meta) end)
```

Called by the ESMTP server after receiving the message data, but before
responding to the client in the live SMTP session.

The message content will be exactly the data passed to the server via the
`DATA` command; no trace or other headers will have been added at this stage.

The event handler will be passed a [Message](../message/index.md) object.

The *conn_meta* parameter represents the connection metadata and
can be used to share state between the various SMTP listener
event handlers. See [Connection Metadata](../connectionmeta.md)
for more information.

If the client issued multiple `"RCPT TO"` commands in the same transaction,
each one will result in a recipient being added to the recipient list in the
message, which you can review and/or modify via
[Message::recipient_list](../message/recipient_list.md) and
[Message::set_recipient](../message/set_recipient.md).

This event is the best place to carry out policy that:

  * Validates/mutates message content/headers, regardless of the recipient list
  * Validates/modifies the recipient list for eg: alias expansion, legal capture

It is NOT recommended to perform recipient-oriented actions at this stage;
instead you should put those in
[smtp_server_message_received](smtp_server_message_received.md) which will be
called once `smtp_server_data` completes.

