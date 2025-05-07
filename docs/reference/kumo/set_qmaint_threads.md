---
tags:
 - threadpool
---

# `kumo.set_qmaint_threads(N)`

{{since('2024.09.02-c5476b89')}}

Sets the number of threads to be used for the qmaint thread pool.
This thread pool is used to perform queue maintenance operations.

The default number of threads is computed using some unspecified fraction of
the available parallelism on the running system, and is shown in the journal on
startup.


```lua
kumo.on('pre_init', function()
  kumo.set_qmaint_threads(12)
end)
```

!!! note
    The `qmaint` used to perform maintenance of a mixture of both
    scheduled and ready queue tasks, but now is used only for
    scheduled queue maintenance. The ready queue maintenance
    is carried out by the `ready_qmaint` thread pool.
    {{since('2025.05.06-b29689af', inline=True)}}
