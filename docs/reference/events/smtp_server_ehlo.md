# `kumo.on('smtp_server_ehlo', function(domain))`

Called by the ESMTP server in response to the client issuing either a `"HELO"`
or `"EHLO"` command.  The event handler is passed the *domain* parameter from
the `HELO/EHLO` command.

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

