---
tags:
 - threadpool
---

# set_signing_threads

```lua
kumo.dkim.set_signing_threads(N)
```

{{since('2024.11.08-d383b033')}}

Sets the number of threads to be used for the `dkimsign` thread pool.  This
thread pool is used to perform DKIM signing, a cryptographic operation that is
CPU intensive.

By default, there is no `dkimsign` pool and signing operations happen in the
context of the calling thread.

Some workloads are a blend of IO and compute, which makes it awkward to
appropriately size the thread pool in the calling context.

In that situation you can call this function to start up the signing thread
pool with an appropriate number of threads like this:

```lua
kumo.on('pre_init', function()
  -- Use half the cores on the system for DKIM signing
  kumo.dkim.set_signing_threads(math.ceil(kumo.available_parallelism() / 2))
end)
```

Using the dkimsign pool adds a bit of overhead in context switching but allows
you to have a larger number of threads in the `smtpsrv` or `readyq` thread
pools to accommodate their more IO-bound workload better.
