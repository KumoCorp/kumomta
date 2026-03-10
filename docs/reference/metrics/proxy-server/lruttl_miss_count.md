# lruttl_miss_count

```
Type: Counter
Labels: cache_name
```
how many times a lruttl cache lookup was a miss for a given cache.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

The `cache_name` label identifies which cache.  See [kumo.set_lruttl_cache_capacity](../../kumo/set_lruttl_cache_capacity.md) for a list of caches.

