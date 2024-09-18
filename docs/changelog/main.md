# Unreleased Changes in The Mainline

## Breaking Changes
* `kcli bounce-list` no longer returns json output by default. Use `--json`
  to explicitly request json output.

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

* The [HTTP injection API](../reference/http/api_inject_v1.md) now supports an
  optional `deferred_spool` parameter that allows deferring writing the
  message(s) spool for a given send attempt, `deferred_generation` for quickly
  accepting a batch for asynchronous generation, and you can control a rate
  limit for ingress using
  [kumo.set_httpinject_recipient_rate_limit](../reference/kumo/set_httpinject_recipient_rate_limit.md).

* We now randomize the set of hosts within a given MX preference level when
  computing the connection plan for an individual session. This helps to
  probablistically load balance across the advertised hosts for the destination.

* The [/api/admin/bounce/v1](../reference/http/api_admin_bounce_v1.md) no longer
  returns *any* partial statistics from the bounce. On systems with very large
  numbers of queues, returning data in this context would take too long. The endpoint
  will now return immediately and report the `id` of the bounce entry, but no
  other statistics.

* `kcli bounce-list` now summarizes the bounce information in a human readable
  format by default, rather than showing the underlying json data as we did
  in previous releases.

* The log_hooks helper now supports batching using the new `batch_size` parameter
  and `send_batch` method. [See the example](../userguide/operation/webhooks.md#batched-hooks)

## Fixes

* `kcli trace-smtp-client` and `kcli trace-smtp-server` would always report
  `0ns` for sessions for which we had not observed the session opening. Now we
  will assume a start time time of the first record observed for a session, so
  that some sense of relative time can be gleaned from the trace output.

* When using the dkim helper and splitting the configuration
  across multiple files, a missing `domain` or `base` configuration section in
  any individual file would raise a validation error.

* Punycode encoded domain names would resolve to the unicode representation of
  the domain name internally, and in the case of domains with no explicit MX
  records, the fallback record that was synthesized from the domain name would
  also be encoded in the unicode representation.

* TSA SuspendTenant rules didn't respect the duration specified in the rule,
  instead defaulting to 5 minutes.
