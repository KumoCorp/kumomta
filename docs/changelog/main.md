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
* Added
  [message:remove_all_named_headers](../reference/message/remove_all_named_headers.md).
  Thanks to @postmastery! #70
* Ready queue names now factor in the delivery protocol, making it easier to vary
  the protocol by eg: *tenant* or *campaign* while keeping the domain the same.
  You will notice a suffix like `@smtp` on the end of queue names in metrics
  and in the `site_name` field of log records as a result of this change.
* It is now more convenient to do smart hosting using the new smtp protocol `mx_list`
  in [kumo.make_queue_config](../reference/kumo/make_queue_config.md).

## Fixes
* Loading secrets from HashiCorp Vault failed to parse underlying json data into
  a byte array.

