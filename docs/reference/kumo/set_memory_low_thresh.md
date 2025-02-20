---
tags:
 - memory
---

# `kumo.set_memory_low_thresh(THRESH)`

{{since('dev')}}

Set the low memory threshold. This usually defaults to 60% of
amount of physical RAM in the system (actually 80% of the *soft limit* which
happens to be 75% of the physical RAM), but ulimit or cgroup constraints may
modify the value.

When the system is using more than the low memory threshold, passive memory
reduction measures will be enabled, including releasing message data when
messages move between queues.

You might want to modify this value if you have a lot of RAM and want the
passive reduction measures to kick in when more of it is in use than the
defaults would otherwise allow.

See [Memory Management](../memory.md) for a discussion on how kumomta manages
memory usage.

It is recommend to set this during the `pre_init` event.

The `THRESH` is expressed as an integer number of bytes.

```lua
kumo.on('pre_init', function()
  kumo.set_memory_low_thresh(800 * 1024 * 1024)
end)
```

