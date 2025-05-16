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

This event can be defined with multiple implementations. The behavior of the
system is that each implementation will be called in the order that they
were defined (the same sequence as the `kumo.on` calls that define them)
until a non-`nil` value is returned.

There is no implicit aggregation or merging of information returned from
`get_egress_path_config`.

The best practice is therefore to only return a value when your implementation
know that it has the definitive information for passed combination of
`routing_domain`, `egress_source` and `site_name`.

This structure allows helper modules to provide information about very specific
egress paths and then allow a final implementation, provided by the system
operator, that can provide a final value for anything else not previously
handled.

This result of means that you need to consider ordering in your `init.lua`; the
various helper modules that you might have imported and configured should
generally have their setup happen prior to your own final definition of the
`get_egress_path_config` event handler.

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
