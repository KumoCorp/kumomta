# kumo.set_max_lua_context_use_count

```lua
kumo.set_max_lua_context_use_count(limit)
```

KumoMTA maintains a pool of lua contexts so that the overhead of evaluating
lua for any given event handler is reduced.

This function allows you to change the maximum number of times that any
given context will be used before discarding it.

The default value is `1024`.

Making it larger increases the potential for cache hits (and thus lower latency),
but increases the potential for increased memory usage.

See also [set_max_lua_context_age](set_max_lua_context_age.md), [set_max_spare_lua_contexts](set_max_spare_lua_contexts.md)
