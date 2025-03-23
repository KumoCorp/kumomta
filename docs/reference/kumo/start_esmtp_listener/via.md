# via

{{since('dev')}}

Define listener-ip-specific parameters.  The value is a cidr-map that is keyed
by the local IP address and whose values are esmtp listener parameters.

!!! note
    This option only really makes sense when used together with
    a wildcard `listen` value of `0.0.0.0` for an IPv4 listener
    or `::` for an IPv6 listener.

This option can be useful to implement IP-based virtual hosting on multi-homed
systems where many IP addresses are served from the same service on the same
port.

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',

  via = {
    -- When clients connect to this server via its 10.0.0.1 IP
    -- address, we will use the hostname and TLS parameters
    -- defined in this block
    ['10.0.0.1'] = {
      hostname = 'mx.example-customer.com',
      tls_certificate = '/path/to/customer1.cert',
      tls_private_key = '/path/to/customer1.key',
    },
    -- When clients connect to this server via its 10.0.0.2 IP
    -- address, we will use the hostname and TLS parameters
    -- defined in this block
    ['10.0.0.2'] = {
      hostname = 'mx.other-customer.com',
      tls_certificate = '/path/to/customer2.cert',
      tls_private_key = '/path/to/customer2.key',
    },
  },
}
```

See also:

 * [smtp_server_get_dynamic_parameters](../../events/smtp_server_get_dynamic_parameters.md)
