---
tags:
 - threadpool
---

# `kumo.set_readyq_threads(N)`

{{since('2024.09.02-c5476b89')}}

Sets the number of threads to be used for the readyq thread pool.
This thread pool is used to perform ready queue maintenance operations.

The default number of threads is computed using some unspecified fraction of
the available parallelism on the running system, and is shown in the journal on
startup.

```lua
kumo.on('pre_init', function()
  kumo.set_readyq_threads(12)
end)
```

