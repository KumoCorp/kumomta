# `message:add_authentication_results(server_id, results)`

{{since('2023.11.28-b5252a41')}}

This method will prepend an `Authentication-Results` header to the message, as
specified by [RFC 8601](https://datatracker.ietf.org/doc/html/rfc8601).

The parameters are:

  * `server_id` - the *authserv-id*.  It is suggested to use
    `msg:get_meta('hostname')` to obtain the hostname that was configured in
    the corresponding SMTP listener.
  * `results` - an array of [authenticationresult](../authenticationresult.md)
    objects holding the results of various authentication methods.

## Example: obtaining DKIM authentication results

```lua
kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Verify the dkim signature and return the results.
  -- Note that this example isn't making any policy decisions;
  -- it is only annotating the message with the results and
  -- allowing it to be relayed
  local auth_results = msg:dkim_verify()

  local spf_result = kumo.spf.check_host(
    msg:sender().domain,
    conn_meta,
    tostring(msg:sender())
  )

  -- add the spf authentication result to our collection of results
  table.insert(auth_results, spf_result.result)

  -- Add the results to the message
  msg:add_authentication_results(msg:get_meta 'hostname', auth_results)
end)
```

might produce an `Authentication-Results` header like this:

```
Authentication-Results: hostname.example.com;
        dkim=pass
        header.a=rsa-sha256
        header.b=jo0EO4dX
        header.d=github.com
        header.i=@github.com
        header.s=pf2023
        spf=pass
```

## See Also:

* [msg:dkim_verify()](dkim_verify.md)
