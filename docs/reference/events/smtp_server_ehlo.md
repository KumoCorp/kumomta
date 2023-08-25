# `kumo.on('smtp_server_ehlo', function(domain, conn_meta))`

Called by the ESMTP server in response to the client issuing either a `"HELO"`
or `"EHLO"` command.  The event handler is passed the *domain* parameter from
the `HELO/EHLO` command.

{{since('2023.08.22-4d895015', indent=True)}}
    The *conn_meta* parameter represents the connection metadata and
    can be used to share state between the various SMTP listener
    event handlers. See [Connection Metadata](../connectionmeta.md)
    for more information.

You may choose to reject the connection via [kumo.reject](../kumo/reject.md).

```lua
-- Called to validate the helo and/or ehlo domain
kumo.on('smtp_server_ehlo', function(domain)
  -- Use kumo.reject to return an error to the EHLO command
  if domain == 'bad.actor' then
    kumo.reject(420, 'go away')
  end
end)
```

