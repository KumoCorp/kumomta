# `kumo.on('get_egress_path_config', function(domain, egress_source, site_name))`

!!! note
    This event handler is in flux and may change significantly

Not the final form of this API, but this is currently how
we retrieve configuration used when making outbound
connections

The `routing_domain` parameter corresponds to the effective `routing_domain` of the
originating *scheduled queue*.  This will be the same as the recipient domain
unless the message had set the `routing_domain` meta value.

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
      enable_tls = 'OpportunisticInsecure',
    }
  end
)
```

See also [kumo.make_egress_path](../kumo/make_egress_path/index.md).
