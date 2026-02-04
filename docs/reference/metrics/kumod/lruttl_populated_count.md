# lruttl_populated_count

```
Type: Counter
Labels: cache_name
```
how many times a lruttl cache lookup resulted in performing the work to populate the entry.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.
