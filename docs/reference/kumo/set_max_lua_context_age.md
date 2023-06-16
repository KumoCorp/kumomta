# `kumo.set_max_lua_context_age(seconds)`

KumoMTA maintains a pool of lua contexts so that the overhead of evaluating
lua for any given event handler is reduced.

This function allows you to change the maximum age of any
given context, measured in seconds, before discarding it.

The default value is `300` (5 minutes).

Making it larger increases the potential for cache hits (and thus lower latency),
but increases the potential for increased memory usage.

See also [set_max_lua_context_use_count](set_max_lua_context_use_count.md),
[set_max_spare_lua_contexts](set_max_spare_lua_contexts.md)
