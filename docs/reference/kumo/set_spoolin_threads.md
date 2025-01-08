---
tags:
 - threadpool
---

# `kumo.set_spoolin_threads(N)`

{{since('2024.09.02-c5476b89')}}

Sets the number of threads to be used for the spoolin thread pool.
This thread pool is used to process spool enumeration during startup.

The default number of threads is computed using some unspecified fraction of
the available parallelism on the running system, and is shown in the journal on
startup.

```lua
kumo.on('pre_init', function()
  kumo.set_spoolin_threads(12)
end)
```

