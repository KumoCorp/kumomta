# rocks_spool_load_shed_active

```
Type: Gauge
Labels: path
```
Set to 1 when this spool's load-shedding gate is latched, 0 otherwise.  When set, ingress paths (SMTP, HTTP inject) reject traffic and foreground store/remove operations fail fast rather than stall.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

The gate latches in either of two ways:

* **Immediate**: a foreground spool operation (load, store,
  remove) returns a rocksdb error classified as definitively
  bad (`Corruption` or `IOError` -- e.g. a missing or corrupt
  SST file discovered during a read).  These conditions have
  no transient interpretation, so the gate latches on the
  first such observation.
* **Debounced**: less specific failure signals --
  `background-errors` has grown since this process started, or
  foreground operations have returned non-fatal errors --
  sustained continuously for the configured
  `error_latch_duration` (default 15s).  This filters out
  brief auto-resumed errors.

If `allow_error_unlatch` is enabled (the default), the gate
auto-clears after `error_unlatch_duration` of observed recovery
(default 5 minutes) with no new errors of either class.
Otherwise it stays set until the process is restarted.

SREs should treat any sustained non-zero value as an
operator-actionable incident; pair this metric with
`rocks_spool_background_errors` to understand why.

