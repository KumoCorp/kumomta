---
title: smtp_server_rcpt_to
---

# `kumo.on('smtp_server_rcpt_to', function(recipient, conn_meta))`

Called by the ESMTP server in response to the client issuing a `"RCPT TO"`
command.  The event handler is passed the *recipient address* parameter from
the `RCPT TO` command.

The *recipient address* is an [EnvelopeAddress](../address/index.md) object.

{{since('2023.08.22-4d895015', indent=True)}}
    The *conn_meta* parameter represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

You may choose to reject the connection via [kumo.reject](../kumo/reject.md).

```lua
kumo.on('smtp_server_rcpt_to', function(recipient)
  if recipient.domain == 'bad.domain' then
    kumo.reject(420, 'not thanks')
  end
end)
```

