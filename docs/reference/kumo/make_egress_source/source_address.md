# source_address

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


