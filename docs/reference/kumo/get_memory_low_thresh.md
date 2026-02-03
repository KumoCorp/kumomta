---
tags:
 - memory
---

# kumo.get_memory_low_thresh

```lua
local thresh = kumo.get_memory_low_thresh()
```

{{since('dev')}}

Returns the low memory threshold, or `nil` if none has been configured.

If your policy doesn't explicitly configure hard/soft limits, the default value depends on the environment into which the process was spawned.

See [kumo.set_memory_low_thresh](set_memory_low_thresh.md) for more details on this specific setting.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

