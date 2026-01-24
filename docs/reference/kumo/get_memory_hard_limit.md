---
tags:
 - memory
---

# kumo.get_memory_hard_limit

```lua
kumo.get_memory_hard_limit(LIMIT)
```

{{since('dev')}}

!!!note
    Unless explicitly set via kumo.set_memory_hard_limit or process is running on an Linux system, this function would return nil

Get the memory_hard_limit configured.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

```lua
  kumo.get_memory_hard_limit()
```
