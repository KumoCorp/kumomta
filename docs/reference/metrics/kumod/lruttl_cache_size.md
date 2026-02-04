# lruttl_cache_size

```
Type: Gauge
Labels: cache_name
```
number of items contained in an lruttl cache.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.
