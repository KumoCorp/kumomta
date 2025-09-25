# ignore_8bit_checks

{{since('dev')}}

When set to `true`, the SMTP client will not pre-emptively consider a message
send attempt as a permanent failure when it would require either `8BITMIME` or
`SMTPUTF8` support to be advertised by the next hop and the next hop does not
advertise the appropriate extension.

The default behavior (when this is set to `false`) is to consider the message
content and the message envelope.

If the content is 8bit and `8BITMIME` is not advertised by the next hop, the
message is not deliverable according to the various SMTP RFCs.  The resolution
to this issue is, in order of preference:

1. Ensure that the generator of the message is using appropriate transfer encoding.
2. Deploy a policy that uses
   [msg:check_fix_conformance](../../message/check_fix_conformance.md) during
   reception to rewrite the message (likely breaking any digital signatures in
   the incoming message).

If the envelope is 8bit and `SMTPUTF8` is not advertised by the next hop, then
there is no way to deliver that message to that destination.  The only way
to successfully deliver such a message (assuming that the recipient is actually
valid) is to ensure that you have configured routing to deliver directly to the
recipient providers domain (eg: don't route via a smart host that doesn't
support `SMTPUTF8`).

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    ignore_8bit_checks = false,
  }
end)
```
