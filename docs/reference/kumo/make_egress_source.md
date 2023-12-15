# `kumo.make_egress_source {PARAMS}`

Defines an *egress source*, which is an entity associated with the source of
outbound traffic from the MTA.  A source must be referenced by a
[pool](make_egress_pool.md) to be useful.

This function is intended to be used inside your
[get_egress_source](../events/get_egress_source.md) event handler.

A source must have at a minimum a *name*, which will be used in logging/reporting.

`PARAMS` is a lua table which may have the following keys:

## name

Required string.

The name of the source. If you call `kumo.define_egress_source` multiple
times with the same name, the most recently defined version of that name will replace
any previously defined source with that name.

```lua
kumo.on('get_egress_source', function(source_name)
  -- Make a source that just has the requested name, but otherwise doesn't
  -- specify any particular source configuration
  return kumo.make_egress_source {
    name = source_name,
  }
end)
```

## source_address

Optional string.

If set, specifies the local IP address that should be used as the source of any
connection that will be made from this source.

If not specified, the kernel will select the IP address automatically.


```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1
    return kumo.make_egress_source {
      name = 'ip-1',
      source_address = '10.0.0.1',
    }
  end
  error 'you need to do something for other source names'
end)
```

!!! note
    When using HA Proxy, the `source_address` will be used when connecting to the proxy.
    You should use `ha_proxy_source_address` to specify the actual address to use
    from the HA Proxy instance to the destination.

## ehlo_domain

Optional string.

If set, specifies the hostname to be passed with the EHLO command when the server connects to a remote host.

If not specified, the kernel will use the server's hostname.

Note that the `ehlo_domain` set by [make_egress_path](make_egress_path.md), if any,
takes precedence over this value.

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1
    kumo.make_egress_source {
      name = 'ip-1',
      source_address = '10.0.0.1',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

## remote_port

Optional integer.

If set, will override the remote SMTP port number. This is useful in scenarios
where your network is set to manage the egress address based on port mapping.

This option takes precedence over
[kumo.make_egress_path().smtp_port](make_egress_path.md#smtp_port).

## ha_proxy_server

Optional string.

If both `ha_proxy_server` and `ha_proxy_source_address` are specified, then
SMTP connections will be made via an HA Proxy server.

`ha_proxy_server` specifies the address and port of the proxy server.

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1, via a proxy server
    kumo.make_egress_source {
      name = 'ip-1',
      ha_proxy_source_address = '10.0.0.1',
      ha_proxy_server = '10.0.0.1:5000',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

## ha_proxy_source_address

Optional string.

Specifies the source address that the HA Proxy server should use when
initiating a connection.

!!! note
   The HA Proxy protocol doesn't provide a mechanism for reporting
   whether binding to this address was successful.  From the perspective
   of KumoMTA, invalid proxy configuration will appear as a timeout
   with no additional context.  We recommend using SOCKS5 instead
   of HA proxy, as the SOCKS5 protocol is better suited for outbound
   connections.

## socks5_proxy_server

{{since('2023.06.22-51b72a83')}}

Optional string.

If both `socks5_proxy_server` and `socks5_proxy_source_address` are specified, then
SMTP connections will be made via a SOCKS5 Proxy server.

`socks5_proxy_server` specifies the address and port of the proxy server.

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1, via a proxy server
    kumo.make_egress_source {
      name = 'ip-1',
      socks5_proxy_source_address = '10.0.0.1',
      socks5_proxy_server = '10.0.0.1:5000',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

## socks5_proxy_source_address

{{since('2023.06.22-51b72a83')}}

Optional string.

Specifies the source address that the SOCKS5 Proxy server should use when
initiating a connection.

## ttl

Optional *time-to-live* specifying how long the source definition should be
cached.  The cache has two purposes:

* To limit the number of configurations kept in memory at any one time
* To enable data to be refreshed from external storage, such as a json data
  file, or a database

The default TTL is 60 seconds, but you can specify any duration using a string
like `"5 mins"` to specify 5 minutes.

