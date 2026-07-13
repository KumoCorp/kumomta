# proxy_connections_completed_total

```
Type: Counter
Labels: listener
```
Total number of proxy sessions that completed successfully.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter increments when a proxy session completes without error,
meaning the client connected, was proxied to the destination, and
both sides closed cleanly.

