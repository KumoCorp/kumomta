# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* You may now use arbitrary bounce classification labels when defining bounce classification rules. #98
* queue helper: Added `setup_with_options` method that allows skipping the registration of the
  `get_queue_config` event handler. This helps when building a more complex configuration
  policy, such as using the rollup helper. Thanks to @cai-n! #101

## Fixes
