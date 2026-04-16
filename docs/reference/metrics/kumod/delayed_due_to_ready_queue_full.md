# delayed_due_to_ready_queue_full

```
Type: Counter
Labels: queue
```
Number of times a message was delayed due to the corresponding ready queue being full.


!!! note
    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.

!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

Delayed in this context means that we moved the message back to its corresponding
scheduled queue with a short retry time, as well as logging a `Delayed` log
record.

Transient spikes in this value indicate normal operation and that the system
is keeping things within your memory budget.

However, sustained increases in this value may indicate that the
[max_ready](../../kumo/make_egress_path/max_ready.md)
configuration for the associated egress path is under-sized for your workload,
and that you should carefully consider the information in
[Budgeting/Tuning Memory](../../memory.md#budgetingtuning-memory)
to decide whether increasing `max_ready` is appropriate, otherwise you risk
potentially over-provisioning the system.

The metric is tracked per `queue` label.  The `queue` is the scheduled
queue name as described in [Queues](../../queues.md).

See [ready_full](ready_full.md) for the equivalent metric tracked
by the ready queue name, which can be helpful to understand which
egress path configuration you might want to examine.

