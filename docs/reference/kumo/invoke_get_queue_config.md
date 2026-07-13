# kumo.invoke_get_queue_config

```lua
local config = kumo.invoke_get_queue_config(queue_name)
```

{{since('2025.01.23-7273d2bc')}}

This function triggers a call through to the
[get_queue_config](../events/get_queue_config.md) event callback(s)
that have been defined in the policy to obtain the effective configuration for
the specified `queue_name`.

The result of that is then serialized and returned as a lua value that has the
same shape as the `PARAMS` defined for
[kumo.make_queue_config](make_queue_config/index.md).

!!! danger
    Take care when using this function and its related `invoke_xxx` functions,
    as you can potentially create cross-dependent, mututally recursive,
    event callbacks that call into each other.
