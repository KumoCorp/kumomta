# kumo.purge_lruttl_cache

```lua
kumo.purge_lruttl_cache(NAME)
```

{{since('dev')}}

Purges (invalidates) all entries from a single named lruttl cache, so that the
next lookup repopulates it from source.

Returns the number of entries that were removed, or `nil` if there is no cache
currently registered under `NAME`.

This is useful when the data backing a specific cache has changed and you want
that change to take effect immediately, without waiting for the cache TTL or a
global config-epoch bump. A config-epoch bump invalidates *every* cache that
was configured with `invalidate_with_epoch = true`, whereas this function
targets only the cache you name. That lets a low-frequency cache be taken off
epoch invalidation (`invalidate_with_epoch = false`) and refreshed explicitly
by whichever component owns its backing data, rather than being invalidated in
lockstep with unrelated, higher-frequency caches.

```lua
kumo.on('some_event', function()
  -- The data backing the "sources_data" cache changed; drop it so that the
  -- next lookup reloads it.
  kumo.purge_lruttl_cache 'sources_data'
end)
```

The equivalent HTTP admin endpoint (kumod) is `POST /api/admin/purge-lruttl-cache`
with a JSON body of `{"name": "NAME"}`.

!!! note
    If more than one live cache shares the same name (which can happen briefly
    while a memoized cache is being replaced after a parameter change), only the
    first matching cache is purged.

## Which caches are affected

This operates on caches registered in the internal lruttl cache registry, which
covers:

* Caches created via `kumo.memoize`.
* Built-in `LruCacheWithTtl` caches, such as the DNS resolver caches.

It does **not** affect caches that are not part of that registry. In
particular, the bounce classifier's caches (used by kumod's delivery-response
classification) are plain LRU caches that are invalidated by config-epoch
changes rather than being registered in the cache registry, so this function
does not touch them.

## See Also

* [kumo.set_lruttl_cache_capacity](set_lruttl_cache_capacity.md)
