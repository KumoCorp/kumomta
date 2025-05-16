# get_egress_pool

```lua
kumo.on('get_egress_pool', function(pool_name) end)
```

Called by the system to determine the composition of a specific named egress pool.

```lua
kumo.on('get_egress_pool', function(pool_name)
  return kumo.make_egress_pool {
    name = pool_name,
    entries = {
      { name = 'ip-1' },
    },
  }
end)
```

See also [kumo.make_egress_source](../kumo/make_egress_source/index.md).

