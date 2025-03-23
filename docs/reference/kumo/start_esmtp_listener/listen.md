# listen

Specifies the local IP and port number to which the ESMTP service
should bind and listen.

Use `0.0.0.0` to bind to all IPv4 addresses.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
}
```

!!! note
    This option cannot be used in dynamic listener contexts such as within
    [via](via.md), [peer](peer.md) or within the parameters returned from
    [smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md).
    It can only be used directly at the top level within the
    `kumo.start_esmtp_listener` call.
