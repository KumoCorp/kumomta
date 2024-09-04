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

* Changed the default [queue
  strategy](../reference/kumo/make_queue_config/strategy.md) to
  `SingletonTimerWheel`.

* `kcli trace-smtp-client` and `kcli trace-smtp-server` both have some new
  options: `--terse` to make it easier to get the sense of the flow of messages
  without seeing all of the message body data, `--only-new` to trace only sessions
  for which the tracer client has observed the session opening, and
  `--only-one` to trace just a single session.

* The HTTP injection API now supports an optional `deferred_spool` parameter
  that allows deferring writing the message(s) spool for a given send attempt.

## Fixes

* `kcli trace-smtp-client` and `kcli trace-smtp-server` would always report
  `0ns` for sessions for which we had not observed the session opening. Now we
  will assume a start time time of the first record observed for a session, so
  that some sense of relative time can be gleaned from the trace output.

* When using the dkim helper and splitting the configuration
  across multiple files, a missing `domain` or `base` configuration section in
  any individual file would raise a validation error.
