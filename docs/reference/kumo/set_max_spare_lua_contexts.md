# `kumo.set_max_spare_lua_contexts(limit)`

KumoMTA maintains a pool of lua contexts so that the overhead of evaluating
lua for any given event handler is reduced.

This function allows you to change the maximum capacity of that pool.

The default value is `8192`.

Make it smaller reduces the amount of memory used while idle, at the cost
of increased latency when the server becomes busy.

See also [set_max_lua_context_use_count](set_max_lua_context_use_count.md),
[set_max_lua_context_age](set_max_lua_context_age.md).
