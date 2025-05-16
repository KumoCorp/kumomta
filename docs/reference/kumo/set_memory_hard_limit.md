---
tags:
 - memory
---

# kumo.set_memory_hard_limit

```lua
kumo.set_memory_hard_limit(LIMIT)
```

{{since('2025.03.19-1d3f1f67')}}

Set the hard limit for memory utilization. This defaults to the amount of
physical RAM in the system.

You typically do not need to modify this value.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

It is recommend to set this during the `pre_init` event.

The `LIMIT` is expressed as an integer number of bytes.

```lua
kumo.on('pre_init', function()
  kumo.set_memory_hard_limit(2 * 1024 * 1024 * 1024)
end)
```
