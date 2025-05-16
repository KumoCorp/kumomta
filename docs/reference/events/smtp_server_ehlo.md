# smtp_server_ehlo

```lua
kumo.on('smtp_server_ehlo', function(domain, conn_meta) end)
```

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
kumo.on('smtp_server_ehlo', function(domain, conn_meta)
  -- Use kumo.reject to return an error to the EHLO command
  if domain == 'bad.actor' then
    kumo.reject(420, 'go away')
  end
end)
```

{{since('2025.01.23-7273d2bc')}}

The signature of the call has been extended to receive the list of
SMTP extensions that will be reported by the EHLO command.
You can optionally return a list of extensions to replace that
list.

This example shows how to advertise support for a hypothetical
"X-SOMETHING" extension:

```lua
kumo.on('smtp_server_ehlo', function(domain, conn_meta, extensions)
  table.insert(extensions, 'X-SOMETHING')
  return extensions
end)
```

If you return `nil` or otherwise omit to return anything, the standard set of
extensions will be returned.

You can filter out or add whatever strings you wish from the `extensions`
parameter, but it is worth noting that what you add/remove from this list has
no impact on what kumomta actually supports, and does not otherwise change its
behavior.

