# `kumo.spf.check_host()`

{{since('2024.11.08-d383b033')}}

This function will check SPF records from DNS for the given domain and IP address.

It takes three arguments:

- `domain` (`string`), the domain to check (for example, from the `smtp_server_ehlo` event)
- `conn_meta` (`connectionmeta`), used to get the client's IP address
- `sender` (optional `string`), the sender address to check

It will return an object containing the SPF `disposition` string and a `result`
of type `authenticationresult` for use with `msg:add_authentication_results()`.

## Example: checking the SPF policy

```lua
kumo.on('smtp_server_ehlo', function(domain, conn_meta)
  -- Check the SPF policy for the domain and return the results.
  local result = kumo.spf.check_host(domain, conn_meta)
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
    "props": {}
  }
]
```

## See Also:

* [msg:add_authentication_results()](../message/add_authentication_results.md)
