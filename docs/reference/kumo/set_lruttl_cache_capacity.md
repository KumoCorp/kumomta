# kumo.set_lruttl_cache_capacity

```lua
kumo.set_lruttl_cache_capacity(NAME, CAPACITY)
```

{{since('2025.03.19-1d3f1f67')}}

Allows you to configure the maximum capacity for a specific named pre-defined cache.

You may only update the capacity for caches defined inside kumomta's Rust code
via `declare_cache!`.  Other caches are assumed to be dynamically created and
expose their capacity as part of their own individual configuration.

```lua
kumo.on('pre_init', function()
  -- Increase the mx cache size from its default of 64*1024 to 128,000
  kumo.set_lruttl_cache_capacity('dns_resolver_mx', 128000)
end)
```

!!! note
    This function is intended to be used in `pre_init`, but it can be called
    at any time.  Reducing the capacity while the cache holds data will trigger
    a partial eviction.  The cache will eventually shrink to conform to the
    new size as the cache is operated upon and background processing trims
    the cache.

Below is a list of pre-defined caches and their default capacities in the
`main` branch.  The list is automatically extracted from the code during the
documentation build, unversioned and may not reflect the version of KumoMTA
that you are running.

{{lruttl_defs()}}
