---
tags:
 - memory
---

# no_memory_reduction_policy

{{since('dev')}}

Specifies what action should be taken when a message is added to the ready
queue corresponding to this egress path when the memory usage is above the *soft
memory limit*.  When the system is in this state, additional active measures
will also be applied to reduce overall memory consumption.

Possible values for this option are:

* `"ShrinkDataAndMeta"` - this is the default (and is the implicit action for
  older versions of KumoMTA).  Both the message data and metadata will be saved
  (if modified since the prior save, or if the message has not yet been saved
  to spool), then both will be released, freeing up the corresponding memory.
* `"ShrinkData"` - Both the message data and metadata will be saved
  (if modified since the prior save, or if the message has not yet been saved
  to spool), then just the message data will be released, freeing up that memory.
  The metadata will be preserved in memory.
* `"NoShrink"` - do not save or free up any message memory.

This setting allows you more control in the trade-off of memory usage against
spool IO pressure. The default is relatively conservative, aiming to avoid OOM
killing at the cost of throughput (increased spool IO). Setting this option to
`"ShrinkData"` or `"NoShrink"` will allow the system to use more memory and
reduce pressure on the spool, but you increase the risk of memory usage
exceeding limits and being targeted by the OOM killer if there is a burst
in your workload.

See also:
 * [low_memory_reduction_policy](low_memory_reduction_policy.md)
 * [Memory Management](../../memory.md)

