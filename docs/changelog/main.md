# Unreleased Changes in The Mainline

## Breaking Changes

 * Rocksdb-backed spool `store()` and `remove()` calls now time out
   after 30 seconds of backpressure rather than blocking
   indefinitely. Tunable via the new
   [store_deadline](../reference/kumo/define_spool/rocks_params.md#store_deadline)
   rocks_params field.

## Other Changes and Enhancements

 * KumoMTA now proactively detects when the rocksdb-backed spool has
   reached a state that requires operator intervention (a missing or
   corrupt SST surfaced through a foreground read/write, or sustained
   background-error accumulation from compactions or flushes) and
   transitions into a load-shedding state. While the spool is
   unhealthy, the SMTP banner returns 421, HTTP injection and
   `/api/check-liveness/v1` return 503, and delivery is paused.
   Pausing delivery limits the window in which a successful SMTP
   transaction could be followed by a failed spool `remove()`, which
   would otherwise cause that message to be redelivered. The
   diagnostic log records each transition that drives this: when
   the rocksdb `background-errors` counter grows, when a foreground
   read or write returns a fatal `IOError` or `Corruption`, when the
   load-shedding gate latches, and (where applicable) when the gate
   later auto-clears after sustained recovery. Each record names
   the spool path and points at the rocksdb LOG file in that
   directory for the underlying cause. The delivery pause itself
   can be toggled with the new
   [kumo.suspend_delivery_when_spool_unhealthy](../reference/kumo/suspend_delivery_when_spool_unhealthy.md)
   policy function (default: enabled). Several new metrics expose
   the underlying state to monitoring:
   [rocks_spool_load_shed_active](../reference/metrics/kumod/rocks_spool_load_shed_active.md),
   [rocks_spool_background_errors](../reference/metrics/kumod/rocks_spool_background_errors.md),
   [rocks_spool_write_stopped](../reference/metrics/kumod/rocks_spool_write_stopped.md),
   [rocks_spool_compaction_pending](../reference/metrics/kumod/rocks_spool_compaction_pending.md),
   [rocks_spool_num_running_compactions](../reference/metrics/kumod/rocks_spool_num_running_compactions.md),
   [rocks_spool_estimate_pending_compaction_bytes](../reference/metrics/kumod/rocks_spool_estimate_pending_compaction_bytes.md),
   and
   [rocks_spool_actual_delayed_write_rate](../reference/metrics/kumod/rocks_spool_actual_delayed_write_rate.md).

 * New [kcli spool-compact](../reference/kcli/spool-compact.md) command
   (and matching `/api/admin/spool-compact/v1` endpoint) forces a flush
   and full-keyspace compaction on a named rocksdb spool. Primarily a
   diagnostic and operational helper; surfaces underlying storage
   errors to the caller.

## Fixes

 * `Message::save_to` was silently discarding errors returned from the
   data and meta spool `store()` operations: the per-spool dirty flags
   were cleared regardless of success, so a message that failed to
   persist was still treated by the SMTP ingress path as accepted.
   Errors now propagate so the ingress path can reject (and the client
   retries) instead of producing a silent loss.
