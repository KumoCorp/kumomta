---
tags:
 - memory
---

# kumo.set_memory_soft_limit

```lua
kumo.set_memory_soft_limit(LIMIT)
```

{{since('2025.03.19-1d3f1f67')}}

Set the *soft limit* for memory utilization. This usually defaults to 75% of
amount of physical RAM in the system, but ulimit or cgroup constraints may
modify the value.

When the system is using more than the soft limit, incoming traffic will be
turned away and various other memory reduction measures will be enabled until
the memory usage falls below the soft limit.

You might want to modify this value if you have a lot of RAM and want to use
more than 75% of it in the common case.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

It is recommend to set this during the `pre_init` event.

The `LIMIT` is expressed as an integer number of bytes.

```lua
kumo.on('pre_init', function()
  kumo.set_memory_soft_limit(1024 * 1024 * 1024)
end)
```

