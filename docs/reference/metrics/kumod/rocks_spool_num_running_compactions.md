# rocks_spool_num_running_compactions

```
Type: Gauge
Labels: path
```
Number of background compactions currently running for this rocksdb instance.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

In a healthy, actively-written spool this is typically non-zero
in bursts.  A value persistently stuck at 0 while
`rocks_spool_compaction_pending` or
`rocks_spool_estimate_pending_compaction_bytes` is growing is a
strong indicator that the background worker is wedged --
cross-reference `rocks_spool_write_stopped` and
`rocks_spool_background_errors`.

