---
tags:
 - memory
---

# kumo.get_memory_soft_limit

```lua
kumo.get_memory_low_limit(LIMIT)
```

{{since('dev')}}

!!!note
    Unless explicitly set via kumo.set_memory_low_limit or kumo.get_memory_hard_limit is set or process is running on an Linux system, this function would return nil

Get the memory_low_limit configured, if not defined by kumo.set_memory_low_limit, this value would default to 80% of memory_soft_limit.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

```lua
  kumo.get_memory_low_limit()
```
