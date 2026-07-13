# proxy_active_connections

```
Type: Gauge
Labels: listener
```
Current number of active proxy connections.


!!! note
    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.

!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This gauge shows the number of connections currently being proxied.
It increments when a connection is accepted and decrements when
the connection closes (successfully or with error).

