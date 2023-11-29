# Authentication Result Object

{{since('2023.11.28-b5252a41')}}

This object represents parsed individual *Authentication Result* as specified by [RFC
8601](https://datatracker.ietf.org/doc/html/rfc8601).  Technically speaking, each
instance of the object corresponds to the parsed form of the `resinfo` term specified
by the RFC.  It doesn't represent an entire header value, just an individual result
for an individual authentication method.

Certain verification methods return arrays of authentication results for you to
act upon in your policy and/or add to incoming message as headers.

The object has the following fields:

  * `method` - required string; the authentication method
  * `method_version` - optional integer; the version of the authentication method
  * `result` - required string; the result of the authentication method.
  * `reason` - optional string; an explanation of why the method produced that result
  * `props` - a table with string keys and values containing various
    method-specific properties that describe additional information about this
    result. For example, for DKIM results, this will often contain copies of
    some of the DKIM signature fields in order to correlate a given result with
    the appropriate DKIM signature header when multiple signatures are present. 

## Example: obtaining DKIM authentication results

This will verify the DKIM signatures that are present in the message
and return an array of authentication results:

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

## See Also:

* [msg:add_authentication_results()](message/add_authentication_results.md)
* [msg:dkim_verify()](message/dkim_verify.md)
