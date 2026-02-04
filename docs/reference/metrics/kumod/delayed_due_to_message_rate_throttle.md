# delayed_due_to_message_rate_throttle

```
Type: Counter
Labels: queue
```
Number of times a message was delayed due to max_message_rate.


!!! note
    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.

!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

Delayed in this context means that we moved the message back to its corresponding
scheduled queue with a short retry time, as well as logging a `Delayed` log
record.

Sustained increases in this value may indicate that the configured
throttles are too severe for your workload, but it is difficult to make
a definitive and generalized statement in these docs without understanding your
workload, policy and the purpose of those throttles.

The metric is tracked per `queue` label.  The `queue` is the scheduled
queue name as described in [Queues](../../queues.md).

