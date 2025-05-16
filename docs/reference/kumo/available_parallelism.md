---
tags:
 - threadpool
---

# kumo.available_parallelism

```lua
kumo.available_parallelism()
```

{{since('2024.06.10-84e84b89')}}

Returns an estimate of the default amount of parallelism a program should use.

Parallelism is a resource. A given machine provides a certain capacity for
parallelism, i.e., a bound on the number of computations it can perform
simultaneously. This number often corresponds to the amount of CPUs a computer
has, but it may diverge in various cases.

Host environments such as VMs or container orchestrators may want to restrict
the amount of parallelism made available to programs in them. This is often
done to limit the potential impact of (unintentionally) resource-intensive
programs on other programs running on the same machine.

## Limitations

The purpose of this API is to provide an easy and portable way to query the
default amount of parallelism the program should use. Among other things it
does not expose information on NUMA regions, does not account for differences
in (co)processor capabilities or current system load, and will not modify the
program’s global state in order to more accurately query the amount of
available parallelism.

Where both fixed steady-state and burst limits are available the steady-state
capacity will be used to ensure more predictable latencies.

Resource limits can be changed during the runtime of a program, therefore the
value is not cached and instead recomputed every time this function is called.
It should not be called from hot code.

The value returned by this function should be considered a simplified
approximation of the actual amount of parallelism available at any given time.
To get a more detailed or precise overview of the amount of parallelism
available to the program, you may wish to use platform-specific APIs as well.
The following platform limitations currently apply to `kumo.available_parallelism()`:

* It may overcount the amount of parallelism available when limited by a
  process-wide affinity mask or cgroup quotas and `sched_getaffinity()` or
  cgroup fs can’t be queried, e.g. due to sandboxing.

* It may undercount the amount of parallelism if the current thread’s affinity
  mask does not reflect the process’ cpuset, e.g. due to pinned threads.

* If the process is in a cgroup v1 cpu controller, this may need to scan
  mountpoints to find the corresponding cgroup v1 controller, which may take
  time on systems with large numbers of mountpoints. (This does not apply to
  cgroup v2, or to processes not in a cgroup.)

* It may overcount the amount of parallelism available when running in a VM
  with CPU usage limits (e.g. an overcommitted host).

*This documentation is excerpted from the underlying [Rust
function](https://doc.rust-lang.org/stable/std/thread/fn.available_parallelism.html).*

!!! note
    If `kumo.available_parallelism` returns an incorrect estimate for your
    system (for the reasons mentioned above), you can force it to return a
    different by value by setting the `KUMO_AVAILABLE_PARALLELISM`
    environment variable to the desired value in the environment when you
    launch `kumod` or `tsa-daemon`.  {{since('2025.05.06-b29689af', inline=True)}}.
