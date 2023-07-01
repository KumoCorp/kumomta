# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
* Calling
  [kumo.configure_redis_throttles](../reference/kumo/configure_redis_throttles.md)
  now also enables redis-based shared connection limits. #41
* [kumo.make_egress_path](../reference/kumo/make_egress_path.md)
  `max_deliveries_per_connection` now defaults to `1024` rather than unlimited.
  Specifying unlimited deliveries is no longer supported as part of shared
  connection limit lease fairness. #41

## Fixes
