---
tags:
 - memory
---

# kumo.get_memory_soft_limit

```lua
local limit = kumo.get_memory_soft_limit()
```

{{since('2026.03.04-bb93ecb1')}}

Returns the soft memory limit, or `nil` if none has been configured.

If your policy doesn't explicitly configure hard/soft limits, whether there is a default soft limit depends on the environment into which the process was spawned.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

