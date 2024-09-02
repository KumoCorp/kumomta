# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
* Queue and Egress configs can now be set to work in a mode where they refresh
  when the underlying configuration files have changed, rather than always
  reloading on a schedule. This makes a difference for sites with many queues.
  This mode is used for the shaping and queue helpers.

* Improved performance of `/metrics` and `/metrics.json` endpoint generation;
  these endpoints now stream data rather than buffering and sending to the
  client. This makes a big difference for systems with over 1 million queues.

* `kcli top` and `kcli queue-summary` now use streaming mertrics parsers to
  improve latency with systems with over 1 million queues.

* HTTP clients can now opt-in to compressed responses by setting their
  `Accept-Encoding` header. We support `gzip` and `deflate` compression.
  Compression is set to `Fastest` rather than `Best`, which produces good
  results without costing too much CPU or request latency.  We do not
  support compressed bodies at this time.

* Optimized per-message overheads in the scheduled queue, reduced the memory
  utilization by 112 bytes per message.

## Fixes

