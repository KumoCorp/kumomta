# rocks_spool_write_stopped

```
Type: Gauge
Labels: path
```
Set to 1 when the rocksdb instance is currently refusing writes at the WriteController layer (memtable count or L0 file count reached the stop threshold), 0 otherwise.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

This reflects rocksdb's own `is-write-stopped` property and
indicates backpressure rather than a fatal background error.
Healthy databases under bursty load may briefly report 1 here.
For the "the database is wedged due to a background error"
signal, see `rocks_spool_load_shed_active` instead.

