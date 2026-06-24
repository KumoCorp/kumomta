# rocks_spool_background_errors

```
Type: Gauge
Labels: path
```
Accumulated count of background errors encountered by the rocksdb instance (failed flushes or compactions, typically caused by I/O errors such as missing or corrupt SST files, ENOSPC, or permission problems).


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

This counter is **monotonic** for the lifetime of the process: it
does not decrease when rocksdb auto-resumes from transient errors
such as a brief ENOSPC.  A non-zero value therefore does not
necessarily mean the database is currently wedged; it means at
least one background error has occurred since the process started.

For SRE monitoring, alert on the **rate of change** (e.g.
`increase(rocks_spool_background_errors[5m]) > 0`) to catch new
occurrences.  For the actionable "the database is wedged right
now and we are shedding load" signal, page on
`rocks_spool_load_shed_active` instead, which combines this
counter, foreground read/write errors, and rocksdb error
severity into a single latched indicator.

