# Unreleased Changes in The Mainline

## Breaking Changes
* [kumo.configure_log_hook](../reference/kumo/configure_log_hook.md) now requires
  a name field to be set to identify which instance of a log hook is being considered
  by the [should_enqueue_log_record](../reference/events/should_enqueue_log_record.md) event.
  This change is to support distributing logs to the traffic shaping automation
  service in addition to feeding them into your own reporting infrastructure.

## Other Changes and Enhancements
* Calling
  [kumo.configure_redis_throttles](../reference/kumo/configure_redis_throttles.md)
  now also enables redis-based shared connection limits. #41
* [kumo.make_egress_path](../reference/kumo/make_egress_path.md)
  `max_deliveries_per_connection` now defaults to `1024` rather than unlimited.
  Specifying unlimited deliveries is no longer supported as part of shared
  connection limit lease fairness. #41

## Fixes
