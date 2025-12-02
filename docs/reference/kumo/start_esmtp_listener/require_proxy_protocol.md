# require_proxy_protocol

{{since('2025.12.02-67ee9e96')}}

!!! danger
    Take care to apply this ONLY in an appropriate `peer` block,
    otherwise you risk a variety of security/authentication related
    bypasses.  Furthermore, this changes the semantics of the SMTP
    session and will prevent non-proxy clients from connecting
    to the server.

When set to `true`, incoming SMTP sessions are required to pass an HA Proxy
Protocol header to override the effective `received_from` and/or `received_via`
connection level metadata items.

Since the proxy protocol header must be unilaterally sent by the client before
the server can return the SMTP banner (which is normally unilaterally sent by
the server), requiring the proxy protocol prevents non-proxy clients from
connecting to the listener when this configuration is in effect.

Both V1 and V2 proxy header packets are supported.

If the proxy header is missing, the connection will be torn down and no service
will be permitted.

After the proxy header is received and successfully parsed, the ESMTP listener
re-evaluates the parameters (especially the [via](via.md) and [peer](peer.md)
blocks), and triggers
[smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md)
to ensure that all the listener configuration has been updated to match the
adjusted `via` and `from` addresses.

```lua
kumo.start_esmtp_listener {
  -- Always use an appropriate `peer` block to scope the
  -- proxy protocol to networks that you trust at the
  -- highest levels
  peer = {
    ['127.0.0.1'] = {
      require_proxy_protocol = true,
    },
  },
}
```

