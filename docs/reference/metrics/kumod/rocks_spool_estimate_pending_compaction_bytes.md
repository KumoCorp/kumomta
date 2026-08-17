# rocks_spool_estimate_pending_compaction_bytes

```
Type: Gauge
Labels: path
```
Estimated total bytes that compaction needs to rewrite to bring all levels back under their target sizes.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

This is a backlog indicator.  Steady-state values depend heavily
on write rate, compression, and the configured compaction style,
so absolute thresholds should be derived from each deployment's
baseline.  Unbounded growth over a multi-hour window indicates
that compaction cannot keep up with the write rate, which
eventually leads to write slowdown
(`rocks_spool_actual_delayed_write_rate` becomes non-zero) and
then to write stop (`rocks_spool_write_stopped` becomes 1).

Only meaningful for level-style compaction.

