# kumo.define_spool

```lua
kumo.define_spool { PARAMS }
```

Defines a named spool storage backend.

KumoMTA uses separate storage areas for metadata and message contents, named
`"meta"` and `"data"` respectively.

This function should be called only from inside your [init](../../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## Spool Parameters { data-search-exclude }
