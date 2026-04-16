# arc_seal

```lua
message:arc_seal(signer, server_id, authentication_results, opt_resolver_name)
```

{{since('2025.12.02-67ee9e96')}}

This method will sign and seal an ARC set to record the current hop as part of
the [Authenticated Received Chain](https://datatracker.ietf.org/doc/html/rfc8617).

The parameters are:

 * `signer` - a signer object created through either
   [kumo.dkim.rsa_sha256_signer](../kumo.dkim/rsa_sha256_signer.md) or
   [kumo.dkim.ed25519_signer](../kumo.dkim/ed25519_signer.md).
 * `server_id` - the hostname to use in the `ARC-Authentication-Results` header
   that is generated as part of the sealing process.
 * `authentication_results` - an array style table holding the set of authentication
   results that should be signed as part of the ARC seal.
 * `opt_resolver_name` parameter is an optional string parameter that specifies
   the name of a alternate resolver defined via
   [kumo.dns.define_resolver](../kumo.dns/define_resolver.md).  You can omit
   this parameter and the default resolver will be used.

Sealing will implicity verify the ARC chain in the message; if that
verification indicates that the chain of custody has been broken, then the seal
operation will return without modifying the message.

!!! note
    Sealing the message MUST occur after all header and body modification,
    otherwise those operations risk invalidating the signatures.



## Example

```lua
kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Collect together various authentication results.
  -- dkim verification returns a possibly empty list
  local results = msg:dkim_verify()
  local arc = msg:arc_verify()
  -- add the arc result to the list we got from dkim
  table.insert(results, arc)
  local spf = kumo.spf.check_msg(msg)
  -- add the spf result to the list we got from dkim
  table.insert(results, spf.result)

  local server_id = msg:get_meta 'hostname'

  -- Add a regular Authentication-Results header for the
  -- sake of consistency with ARC
  msg:add_authentication_results(server_id, results)

  -- Set up a signer; this is just an example that loads
  -- a key from a file on disk.
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }

  -- Emits an ARC-Authentication-Results header,
  -- computes an ARC-Message-Signature header based on the settings
  -- in the signer, and then computes a final ARC-Seal header
  -- to seal the ARC chain of custody. Those 3 headers are
  -- added to the message.
  msg:arc_seal(signer, server_id, results)
end)
```


