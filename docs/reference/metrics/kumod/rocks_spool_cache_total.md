# rocks_spool_cache_total

```
Type: Gauge
Labels: path
```
Approximate memory (bytes) usage by cache.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This may be useful when understanding the memory usage of
the system.

