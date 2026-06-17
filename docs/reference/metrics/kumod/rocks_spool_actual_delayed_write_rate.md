# rocks_spool_actual_delayed_write_rate

```
Type: Gauge
Labels: path
```
Current delayed write rate (bytes/second) applied by rocksdb to throttle foreground writers.  0 means no slowdown is in effect.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

A non-zero value means rocksdb is intentionally slowing writers
down because compaction or flush is falling behind.  This is the
early-warning signal that precedes a full write stop: if this
remains non-zero for an extended period, investigate the
compaction backlog
(`rocks_spool_estimate_pending_compaction_bytes`) and underlying
disk throughput before the database transitions to
`rocks_spool_write_stopped == 1`.

