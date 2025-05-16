---
title: get_egress_pool
---

# `kumo.on('get_egress_pool', function(pool_name))`

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

