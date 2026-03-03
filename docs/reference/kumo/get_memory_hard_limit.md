---
tags:
 - memory
---

# kumo.get_memory_hard_limit

```lua
local limit = kumo.get_memory_hard_limit()
```

{{since('dev')}}

Returns the hard memory limit, or `nil` if none has been configured.

If your policy doesn't explicitly configure a limit via [kumo.set_memory_hard_limit](set_memory_hard_limit.md), whether there is a hard limit depends on the environment into which the process was spawned.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

