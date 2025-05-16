---
tags:
 - threadpool
---

# kumo.set_ready_qmaint_threads

```lua
kumo.set_ready_qmaint_threads(N)
```

{{since('2025.05.06-b29689af')}}

Sets the number of threads to be used for the `readyq_maint` thread pool.
This thread pool is used to perform ready queue maintenance operations.

The default number of threads is computed using some unspecified fraction of
the available parallelism on the running system, and is shown in the journal on
startup.


```lua
kumo.on('pre_init', function()
  kumo.set_ready_qmaint_threads(12)
end)
```

