---
tags:
 - threadpool
---

# `kumo.set_logging_threads(N)`

{{since('2024.09.02-c5476b89')}}

Sets the number of threads to be used for the logging thread pool.
This thread pool is used to process logging.

The default number of threads is computed using some unspecified fraction of
the available parallelism on the running system, and is shown in the journal on
startup.

```lua
kumo.on('pre_init', function()
  kumo.set_logging_threads(12)
end)
```

