# rocks_spool_load_shed_active

```
Type: Gauge
Labels: path
```
Set to 1 while this spool refuses writes, or 0 otherwise. When set, SMTP and HTTP ingress reject traffic, and store/remove operations return an error immediately.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('2026.09.22-a276d4a8')}}

A foreground operation returning `Corruption` or `IOError` immediately
latches the gate, causing subsequent writes to return errors. These failures
include missing and corrupt SST files.

Newly observed background errors, other foreground errors, and timeouts
while waiting for RocksDB to accept a write start the `error_latch_duration`
delay (default 15 seconds). Even an isolated error causes a latch after
this delay.

With `allow_error_unlatch = true` (the default), writes resume after
`error_unlatch_duration` (default 5 minutes) has elapsed since the later
of the latch time and the most recent error observation. If the database
remains damaged, another error can latch the gate again. Set
`allow_error_unlatch = false` to keep writes paused until an operator
inspects the database and restarts the process.

The monitor checks background-error growth and applies the latch and
retry timers, then sleeps for 5 seconds. Later errors are handled by
the next iteration.

Automatic retries accept this sampling delay: writes may resume between a
background error and its observation. Disable `allow_error_unlatch` to
keep writes paused across that window.

If writes remain paused, inspect `rocks_spool_background_errors` and the
RocksDB LOG to identify the storage failure.

