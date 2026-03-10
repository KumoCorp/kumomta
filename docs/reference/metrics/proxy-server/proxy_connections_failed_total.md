# proxy_connections_failed_total

```
Type: Counter
Labels: listener
```
Total number of connections that failed during handshake or proxying.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter increments when a connection fails due to handshake errors,
authentication failures, timeouts, or I/O errors during proxying.

