# `kumo.on('get_egress_source', function(source_name))`

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    -- other fields here
  }
end)
```

See also [kumo.make_egress_source](../kumo/make_egress_source/index.md).
