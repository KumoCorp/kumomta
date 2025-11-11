# arc_verify

```lua
local result = message:arc_verify(OPT_RESOLVER_NAME)
```

{{since('dev')}}

This method will verify the [Authenticated Received
Chain](https://datatracker.ietf.org/doc/html/rfc8617) in the message, returning
an [AuthenticationResult](../authenticationresult.md) object indicating the
status of the verification.

The `result` field in the returned authentication result can have one
of the following values, as specified by RFC8617:

 * `none` - no ARC sets were present in the message
 * `pass` - all ARC sets validated, the chain of custody is intact
 * `fail` - something prevented validating the chain of custody. The `reason`
   field will offer a (potentially partial) explanation.

There are a number of checks associated with validating the ARC chain of
custody, some of which will compound and cause subsequent checks to fail.
Interally the ARC validator tracks all of these but will only expose the first
such failure in the `reason` field of the `AuthenticationResult` object for the
sake of brevity.

The `OPT_RESOLVER_NAME` parameter is an optional string parameter that
specifies the name of a alternate resolver defined via
[kumo.dns.define_resolver](../kumo.dns/define_resolver.md).  You can omit this
parameter and the default resolver will be used.

## Example: obtaining the status

```lua
kumo.on('smtp_server_data', function(msg, conn_meta)
  local arc = msg:arc_verify()
  kumo.log_info('ARC result', kumo.serde.json_encode_pretty(arc))
  if arc.result == 'fail' then
    -- Please note that this is technically a "BAD" example, as
    -- RFC8617 says: a message with an Authenticated Received Chain
    -- with a Chain Validation Status of "fail" MUST be treated the
    -- same as a message with no Authenticated Received Chain
    kumo.reject(
      550,
      '5.7.29 ARC Validation failure: ' .. tostring(arc.reason)
    )
  end
end)
```
