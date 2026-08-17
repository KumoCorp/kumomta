# rocks_spool_compaction_pending

```
Type: Gauge
Labels: path
```
Set to 1 when at least one compaction is pending for this rocksdb instance, 0 otherwise.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

Brief flapping is normal under write load.  A value of 1 that
persists alongside `rocks_spool_num_running_compactions == 0` is
suspicious and suggests the compaction worker is not making
progress.

