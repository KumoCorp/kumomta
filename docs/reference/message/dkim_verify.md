# `message:dkim_verify()`

{{since('2023.11.28-b5252a41')}}

This method will verify each DKIM signature that is present at the top level of
the message, up to a limit of 10 signatures.  The limit is in place to limit the
scope of a DoS attack being carried out through maliciously constructed messages.

For each signature, an [authenticationresult](../authenticationresult.md) object
will be constructed and an array of those results will be returned to the caller.

## Example: obtaining DKIM authentication results

```lua
kumo.on('smtp_server_message_received', function(msg)
  -- Verify the dkim signature and return the results.
  -- Note that this example isn't making any policy decisions;
  -- it is only annotating the message with the results and
  -- allowing it to be relayed
  local verify = msg:dkim_verify()
  print('dkim', kumo.json_encode_pretty(verify))
  -- Add the results to the message
  msg:add_authentication_results(msg:get_meta 'hostname', verify)
end)
```

might print something like this to the diagnostic log:

```
dkim    [
  {
    "props": {
      "header.d": "github.com",
      "header.i": "@github.com",
      "header.s": "pf2023",
      "header.a": "rsa-sha256",
      "header.b": "jo0EO4dX"
    },
    "result": "pass",
    "method": "dkim",
    "reason": null,
    "method_version": null
  }
]
```

and produce an `Authentication-Results` header:

```
Authentication-Results: hostname.example.com;
        dkim=pass
        header.a=rsa-sha256
        header.b=jo0EO4dX
        header.d=github.com
        header.i=@github.com
        header.s=pf2023
```

## See Also:

* [msg:add_authentication_results()](add_authentication_results.md)
