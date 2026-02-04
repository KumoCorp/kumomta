# ready_full

```
Type: Counter
Labels: service
```
number of times a message could not fit in the ready queue.


!!! note
    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.

!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

See [delayed_due_to_ready_queue_full](delayed_due_to_ready_queue_full.md)
for the equivalent metric tracked by scheduled queue name, as well as
a discussion on what this event means.

