# Unreleased Changes in The Mainline

## Breaking Changes
* `kcli bounce-list` no longer returns json output by default. Use `--json`
  to explicitly request json output.
* The filename format for log file segments now includes fractional seconds
  so that there is no chance of file naming collision when using aggressively
  small values for `max_file_size` or `max_segment_duration`.
* TSA rules will no longer by default match internally generated failure
  responses, that is, those that begin with the text `KumoMTA internal: `.
  This prevents accidentally triggering cyclical behavior in the case where
  you have a very lenient regex for a rule that triggers a suspension.
  If you have rules that you wish to intentionally match these internal
  messages, you can mark the automation entry with `match_internal = true`
  to allow the match to be considered.

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

* Added `proxy_connection_failures` and `bind_failures` counters to track
  the number of times that kumod either failed to connect to an egress proxy
  server, or failed to directly bind a source address. Both of these events
  typically indicate a severe issue with the local infrastructure, either in
  terms of a configuration error or production service availability.

* The `validate-shaping` utility and underlying
  [kumo.shaping.load](../reference/kumo.shaping/load.md) function now support
  options to control whether individual checks should be warnings, errors or be
  ignored.  The shaping helper allows specifying these options as
  `load_validation_options` (for regular load-time checks) and
  `validation_options` (for `--validate` mode).

* The [requeue_message](../reference/events/requeue_message.md) event now also
  receives the SMTP response that led to the requeue event being triggered.

* The bounce classifier will now automatically reload when the configuration
  epoch is updated. #298

* Spool enumeration completion no longer prevents the reception of new messages.
  The spool does need to have started before messages will be received, which
  means that there is still a very small window during startup where the liveness
  check and SMTP sessions can turn away incoming connections.

* The dkim helper now supports passing the `expiration` value through to
  the underlying signer.

* HTTP injection now includes control over the
  [trace_headers](../reference/http/api_inject_v1.md#trace_headers), which now
  default to including the supplemental trace header for
  FBL/ARF processing, but not including the Received header.
  These parameters are set per-request.

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

* Suspensions didn't apply to every case where a message could enter the ready
  queue.  We now check both as part of entering the ready queue and as a last
  moment check when a message is popped off the ready queue. We will now log a
  `TransientFailure` log record whenever a message matches a scheduled queue
  suspension. In addition, suspensions will now always respect the normal
  exponential backoff retry schedule instead of clumping together when the
  suspension expires. #290 #293

* During a low memory condition, we'd only release the body and metadata memory
  if they had previously been saved, when the intent was that we should
  explicitly save it and then drop it.  This meant that running with
  deferred-spooling or otherwise modifying the message or its metadata after
  reception could result in messages that wouldn't be eligible to shrink
  until after their next attempt. In addition, we could repeatedly try this
  each time the readyq maintainer would trigger during a memory shortage.

* The [requeue_message](../reference/events/requeue_message.md) event was
  internally named `message_requeued`, contrary to the documentation. This has
  now been corrected. #236

* The [throttle_insert_ready_queue](../reference/events/throttle_insert_ready_queue.md) event
  was not correctly registered and would never trigger.

* A MIME message rebuild could improperly re-encode unicode Subject lines into
  a series of quoted-printable encoded-words, causing spaces between those
  words to be effectively discarded when the subject is decoded.  The header
  re-encoding will now prefer to re-assemble unstructured fields as a single
  encoded-word to avoid this.

* Using `msg:append_text_html()` or `msg:append_text_plain()` on a mime part
  that had a pre-existing `Content-Transfer-Encoding` header wouldn't remove
  the header. If the original encoding was `base64` and the new form of the
  part was written out in `quoted-printable` then the resulting mime part would
  be ambiguous to decode.
