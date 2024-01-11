# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

## Fixes

* The `delivered_this_connection` counter was incorrectly double-counted for
  SMTP sessions, effectively halving the effective value of
  `max_deliveries_per_connection`.
* Re-run the ready queue maintainer immediately after closing a connection
  due to reaching the `max_deliveries_per_connection`, so that new connection(s)
  can be established to replace the one that just closed. Previously, we would
  only do this once every minute. #116
* The `smtp_client_rewrite_delivery_status` event could trigger with incorrect
  scheduled queue name components.

