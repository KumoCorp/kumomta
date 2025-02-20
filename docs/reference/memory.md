---
tags:
 - memory
---

# Memory Management

KumoMTA makes aggressive use of memory in the interest of performance,
but memory is a finite resource.

This section of the documentation discusses the general strategies
employed by KumoMTA when it comes to managing its working set.

## Memory Limits and Headroom

On startup KumoMTA will determine the maximum RAM that is available
but checking the following things in this same sequence:

* A `cgroup` memory limit, checking v2 cgroups before v1 cgroups.
* A `ulimit` constraint as observed via `getrlimit(2)`.
* The physical RAM available to the system

A threshold is calculated at 75% of whichever of the above constraints
is detected first of all.

The calculated limit is published via the prometheus metrics endpoint
as `memory_limit`, and is reported in bytes.

A background task is started to monitor the memory usage of the system
which is derived from:

* If running in a cgroup, the usage reported by that cgroup. Note that
  this may include memory used by other processes in that same cgroup.
* The Resident Set Size (RSS) as reported in `/proc/self/statm`
* The value configured via
  [kumo.set_memory_soft_limit](kumo/set_memory_soft_limit.md), if any,
  will always take precedence over the above.

The current usage is published via the prometheus metrics endpoint
as `memory_usage` and is reported in bytes.

### Headroom and Load Shedding

The *memory headroom* of the system is defined as the current value of
`memory_limit - memory_usage`, clamping negative numbers to 0.

If the memory headroom hits 0, at the point of transitioning from non-zero to
zero, the system will take measures to scale back memory usage:

* Each *ready queue* will be walked and each message will be subject to
  a *shrink* operation that will ensure that the message body is journalled
  to spool (if using deferred spooling) and then free up the message body
  memory.
* Each LRU cache in the system will be purged. The DNS subsystem, the
  memoize function and DKIM signer caches are commonly used examples
  of LRU caches
* If using RocksDB for the spool, RocksDB will be asked to flush any memtables
  and caches.  You can monitor the usage of these objects via the
  `rocks_spool_mem_table_total` and `rocks_spool_mem_table_readers_total`
  prometheus metrics.

While the system is operating with a memory headroom of 0 the liveness
check will indicate that it is unhealthy and neither the SMTP or HTTP
listeners will accept any new incoming messages.

Once the system recovers and the headroom increases above zero, incoming
messages will again be accepted and delivered.

### Passive Measures

In addition to the active measures when headroom reaches zero, there
are a couple of passive measures:

* The various thread pools in the system continuously signal to the jemalloc
  allocator when they are idle, allowing memory to returned from its per-thread
  caches and to make it available to other threads, or to be reclaimed.

* When the `memory_usage` is >= 80% of the `memory_limit`, messages moving into
  a ready queue, or being freshly inserted, will be subject to the same shrink
  operation described above.  You may configure this value via
  [kumo.set_memory_low_thresh](kumo/set_memory_low_thresh.md).

## Budgeting/Tuning Memory

Assuming ideal conditions, where the rate of egress is equal to the rate of
ingress, the primary contributor to core memory usage is message bodies in the
*ready queue*.

The default fast path is that an incoming message is received and its body
is retained in RAM until after the first delivery attempt.

If your average message size is 100kb and you have `max_ready = 1000` then you
are effectively budgeting `1000 x 100kb` of RAM for a given ready queue in its
worst case.

If you have `max_ready` set very large by default then you can increase memory
pressure in the case where you have an issue with the rate of egress.

In general you should size `max_ready` to be just large enough to accommodate
your sustained egress rate for a given queue.  For example, if you have a
throughput of `1000/s` then you might want to set `max_ready` to approximately
`2000` in order to avoid transient delays if the ready queue is filled up.  The
precise value for your system might be a slightly different single digit
multiple of the per-second rate; this number is just a ballpark suggestion.

Conversely, if your *maximum* egress rate is `1000/s` and you have
over-provisioned `max_ready` to a very large number like `100,000`, and you
have an issue where your egress rate drops to zero, then you will be allowing
the system to use up to 50x as much memory as your normal rate of throughput
would need.  If you have multiple queues over-provisioned in the same way, the
system will be placed under a lot of memory pressure.

The recommendation is to keep `max_ready` at a reasonably small value by
default, but to increase it for your top-5 or top-10 destination sites by
egress rate in order to achieve the sweet spot in throughput.
