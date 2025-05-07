# peer

{{since('2025.05.06-b29689af')}}

Define peer-specific parameters.  The value is a cidr-map that is keyed
by the peer address and whose values are esmtp listener parameters.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',

  -- This is the banner that will be used by default
  banner = 'Welcome to KumoMTA!',

  peer = {
    -- Clients connecting from the loopback address
    -- will match this section, and the values defined
    -- within will override any values defined in the
    -- base set of parameters passed directly to start_esmtp_listener.
    ['127.0.0.1'] = {
      -- So they will have this customized banner
      banner = 'Welcome loopback!',
    },
    -- Similarly, clients connecting from any address
    -- in the range 192.168.1.0 through 192.168.1.255
    -- will match this block
    ['192.168.1.0/24'] = {
      banner = 'Welcome LAN!',
    },
  },
}
```

See also:

 * [smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md)
