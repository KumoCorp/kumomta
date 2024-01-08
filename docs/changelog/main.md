# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

## Fixes

* The `delivered_this_connection` counter was incorrectly double-counted for
  SMTP sessions, effectively halving the effective value of
  `max_deliveries_per_connection`.
