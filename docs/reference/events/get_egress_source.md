# get_egress_source

```lua
kumo.on('get_egress_source', function(source_name) end)
```

Called by the system to determine the composition of a specific named egress
source.

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    -- other fields here
  }
end)
```

See also [kumo.make_egress_source](../kumo/make_egress_source/index.md).
