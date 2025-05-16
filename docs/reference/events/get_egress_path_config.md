# get_egress_path_config

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name) end
)
```

The `routing_domain` parameter corresponds to the effective `routing_domain` of
the originating *scheduled queue*.  This will be the same as the recipient
domain unless the message had set the `routing_domain` meta value.

An implementation for this event can be provided only once.

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
