# smtp_server_get_dynamic_parameters

```lua
kumo.on(
  'smtp_server_get_dynamic_parameters',
  function(listener, conn_meta) end
)
```

{{since('2025.05.06-b29689af')}}

!!! note
    This option is primarily intended to be used together with
    a wildcard `listen` value of `0.0.0.0` for an IPv4 listener
    or `::` for an IPv6 listener where you desire to dynamically
    configure IP based virtual MTA service.

Called by the ESMTP server when a new server session has accepted
a connection from a client, and offers a chance to update the
configuration for the listener dynamically.  This event triggers
before [smtp_server_connection_accepted](smtp_server_connection_accepted.md).

The parameters are:

* `listener` - the stringified version of the listener address, such as `0.0.0.0:25`
* `conn_meta` - the [Connection Metadata](../connectionmeta.md) object

The return value must be a table holding ESMTP listener parameter *overrides*
that you wish to apply to the existing listener parameters for this connection.
The fields that you specify in the return value will override the fields that
were already configured.  Almost every field described under
[kumo.start_esmtp_listener](../kumo/start_esmtp_listener/index.md) can be used;
those that cannot will indicate it in their individual documentation pages.

The following example is equivalent to the
[via](../kumo/start_esmtp_listener/via.md) example, except that rather than the
`via` parameters being statically configured during the `init` event, they are
computed for every new connection:

```lua
kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',
  }
end)

kumo.on('smtp_server_get_dynamic_parameters', function(listener, conn_meta)
  return {
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
end)
```

This late binding of the configuration can be used to aid in dynamically
updating the set of listeners on a wildcard port.

For example, you could put the listener overrides into a TOML or JSON
file that has the same shape as the listener parameters:

```toml
[via.'10.0.0.1']
hostname = 'mx.example-customer.com'
tls_certificate = '/path/to/customer1.cert'
tls_private_key = '/path/to/customer1.key'

[via.'10.0.0.2']
hostname = 'mx.other-customer.com'
tls_certificate = '/path/to/customer2.cert'
tls_private_key = '/path/to/customer2.key'
```

Then change the policy code to load it:

```lua
kumo.on('init', function()
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:25',
  }
end)

kumo.on('smtp_server_get_dynamic_parameters', function(listener, conn_meta)
  return kumo.serde.toml_load '/opt/kumometa/etc/policy/listener_params.toml'
end)
```

Depending upon the size of the data you are loading, and especially if you
choose to load data from an external service, you should consider using
[kumo.memoize](../kumo/memoize.md) to cache the data.
