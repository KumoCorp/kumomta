# kumo.invoke_get_egress_pool

```lua
local config = kumo.invoke_get_egress_pool(pool_name)
```

{{since('dev')}}

This function triggers a call through to the
[get_egress_pool](../events/get_egress_pool.md) event callback(s)
that have been defined in the policy to obtain the effective configuration for
the specified `pool_name`.

The result of that is then serialized and returned as a lua value that has the
same shape as the `PARAMS` defined for
[kumo.make_egress_pool](make_egress_pool/index.md).

This function may be satisfied by the internal cache of resolved (and
validated!) pool configuration information, so it may not directly
trigger the `get_egress_pool` callback every time that it is called.

!!! danger
    Take care when using this function and its related `invoke_xxx` functions,
    as you can potentially create cross-dependent, mututally recursive,
    event callbacks that call into each other.
