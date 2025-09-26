# allow_xclient

{{since('dev')}}

!!! danger
    Take care to apply this ONLY in an appropriate `peer` block,
    otherwise you risk a variety of security/authentication related
    bypasses.

When set to `true`, allows the connected session to use the
[XCLIENT](https://www.postfix.org/XCLIENT_README.html) ESMTP extension.

`XCLIENT` is used primarily in testing environments to facilitate validation of
authentication checks that are based upon the IP address of the peer or the
server itself.

KumoMTA supports the following XCLIENT attributes:

 * `ADDR` and `PORT`: cause the `received_from` metadata to change
   to reflect the specified address and/or port.
 * `DESTADDR` and `DESTPORT`: cause the `received_via` metadata to
   change to reflect the specified address and/or port.

None of the other XCLIENT attributes are supported at the time of writing.

After `XCLIENT` has been successfully negotiated, the ESMTP listener
re-evaluates the parameters (especially the [via](via.md) and [peer](peer.md)
blocks), and triggers
[smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md)
to ensure that all the listener configuration has been updated to match the
adjusted `via` and `from` addresses.

```lua
kumo.start_esmtp_listener {
  -- Always use an appropriate `peer` block to scope XCLIENT to
  -- networks that you trust at the highest levels
  peer = {
    ['127.0.0.1'] = {
      allow_xclient = true,
    },
  },
}
```


