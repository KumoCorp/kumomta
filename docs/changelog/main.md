# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* You may now use arbitrary bounce classification labels when defining bounce classification rules. #98
* queue helper: Added `setup_with_options` method that allows skipping the registration of the
  `get_queue_config` event handler. This helps when building a more complex configuration
  policy, such as using the rollup helper. Thanks to @cai-n! #101
* You may now use simple suffix based wildcards like `X-*` to match header
  names to capture in log records. See
  [kumo.configure_local_logs](../reference/kumo/configure_local_logs.md). #74

## Fixes

* MTA-STS policy may fail to match due to trailing periods in mx host names
