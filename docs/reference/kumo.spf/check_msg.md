# kumo.spf.check_msg

```lua
kumo.spf.check_msg(MESSAGE, OPT_RESOLVER_NAME)
```

{{since('dev')}}

This function will check SPF records from DNS for the provided message.
It will extract the appropriate domain and sender information from the metadata and message.

It will return an object containing the SPF `disposition` string and a `result`
of type `authenticationresult` for use with `msg:add_authentication_results()`.

The `OPT_RESOLVER_NAME` parameter is an optional string parameter that
specifies the name of a alternate resolver defined via
[kumo.dns.define_resolver](../kumo.dns/define_resolver.md).  You can omit this
parameter and the default resolver will be used.

## Example: checking the SPF policy

```lua
kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Check the SPF policy for the domain and return the results.
  local result = kumo.spf.check_msg(msg)
  print('spf', kumo.json_encode_pretty(result))
  if result.disposition ~= 'pass' then
    kumo.reject(420, 'go away')
  end
end)
```

might print something like this to the diagnostic log:

```
spf    [
  "disposition": "pass",
  {
    "result": "pass",
    "method": "spf",
    "reason": "matched 'all' directive",
    "method_version": null,
    "props": {
        "smtp.mailfrom": "sender@example.com"
    }
  }
]
```

## See Also:

* [msg:add_authentication_results()](../message/add_authentication_results.md)

