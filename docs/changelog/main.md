# Unreleased Changes in The Mainline

## Breaking Changes
 * The `rfc5321_rustls_config` cache has been renamed to `rustls_client_config`.
   If you have a policy that tunes this cache via
   [kumo.set_lruttl_cache_capacity](../reference/kumo/set_lruttl_cache_capacity.md),
   you will need to update the cache name.
 * The effect of the [skip_hosts](../reference/kumo/make_egress_path/skip_hosts.md)
   configuration has been downgraded from a `550` to a `451` to make it more inline
   with the effect of resolving a domain to an empty list of MX hosts.  The rationale
   for this is that users primarily employ `skip_hosts` to prevent the use of IPv6.
   That, coupled with a few recent issues with Microsoft hosted domains where their
   DNS would only transiently return IPv6 addresses (no IPv4) addresses meant that
   some mail could be inadvertently permanently failed.  The reason field now
   also has `KumoMTA internal:` prefixed to it, to make it clearer that it was
   synthesized by us, rather than returned from a remote host.

## Other Changes and Enhancements
 * Enhanced [Access Control](../reference/access_control.md) subsystem,
   supported by a new Authentication, Authorization and Accounting (AAA) module
   exposed to lua as [kumo.aaa](../reference/kumo.aaa/index.md).
 * The `Handlebars` template dialect now runs with recursive lookup
   for improved compatibility with other handlebars implementations.
 * `msg:check_fix_conformance()` can now detect and attempt to fix issues where
   the charset is invalid for parts that use transfer-encoding, by applying
   any charset detection options, falling back to UTF-8.
 * The lua HTTP `Request` object now supports AWS V4 signatures via a new
   [request:aws_sign_v4](../reference/kumo.http/Request.md#requestaws_sign_v4params)
   method.  Thanks to @AdityaAudi! #458
 * The [HTTP Injection API](../reference/http/kumod/api_inject_v1_post.md) and [MIME
   Builder API](../reference/kumo.mimepart/builder.md) now support creating
   messages with [AMP
   HTML](https://amp.dev/documentation/guides-and-tutorials/email/learn/email-spec/amp-email-structure)
   parts.
   [mimepart:get_simple_structure()](../reference/mimepart/get_simple_structure.md)
   also supports AMP HTML parts.
 * Improved the context shown in error messages produced by the HTTP injection
   API
 * Kumo Proxy:
     * Now optionally supports configuration via a proxy policy lua script.
     * Optional support for TLS and mutual TLS when using a proxy policy script,
       however, kumod itself doesn't currently support using TLS for SOCKS5.
     * Optional support for RFC 1929 authentication
     * Use [proxy.start_proxy_listener](../reference/proxy/start_proxy_listener/index.md)
       function to configure a SOCKS5 proxy server
     * Many thanks to @vietcgi! #459
     * Exposes [proxy-specific metrics](../reference/metrics/proxy-server/index.md)
       via its new [proxy.start_http_listener](../reference/proxy/start_http_listener.md).
       Thanks to @AdityaAudi! #472
 * New [kumo.xfer.xfer](../reference/kumo.xfer/xfer.md) and
   [kumo.xfer.xfer_in_requeue](../reference/kumo.xfer/xfer_in_requeue.md)
   functions to enable per-message transfer between nodes, which is useful in
   combination with the
   [requeue_message](../reference/events/requeue_message.md) event.
 * New
   [message:increment_num_attempts](../reference/message/increment_num_attempts.md)
   method for advanced message manipulation.
 * The [requeue_message](../reference/events/requeue_message.md) event now
   exposes additional context about the event leading to the the requeue,
   allowing for more nuanced/advanced requeue logic.
 * Each metric exported by kumod now has a documentation page. You can find an
   index at [kumod metrics](../reference/metrics/kumod/index.md).
 * New `/tsa/status` HTTP endpoint for the TSA daemon which can be used to determine
   that its service is up.
 * New
   [redis_operation_latency](../reference/metrics/kumod/redis_operation_latency.md)
   histogram metric which tracks operation type, status and latency.
 * New system- and process-specific CPU usage metrics. We export both the
   total overall percentage across all cores, which results in values ranging
   from `0%` through to `num_cpus * 100%` for a fully saturated system,
   as well as normalized values that use `100%` to indicate a fully saturated
   system.  The process-specific variants account only for the individual service
   process (eg: `kumod` only), whereas the system-specific variants indicate
   the total load on the entire system. #186

## Fixes

 * An SPF record containing U+200B (zero width space) could cause
   SPF record parsing to panic and the service to crash
 * MIME part body extraction did not always consider the charset for text parts
 * Errors raised while dispatching
   [should_enqueue_log_record](../reference/events/should_enqueue_log_record.md)
   were not logged to the diagnostic log.
 * Rebuilding (eg: for conformance fixing via `msg:check_fix_conformance()`, or
   as part of the post-HTTP injection fixup) a header like `From:
   "something\n\tthat wraps lines" <user@example.com>` would produce an invalid
   rendition of that header.
 * Setting `content.headers["To"]` in the HTTP injection API would result in
   two `To` headers being generated in the message; one for the per-recipient
   `To` header, and one for the specified `content.headers["To"]` value.  This
   has been fixed; the behavior now is to use the `content.headers["To"]`
   header and not to emit a per-recipient `To` header in this situation.
 * HTTP Injection didn't gate on the spool being started which meant that
   there was a race condition on startup where an injection request could
   begin processing prior to starting spool enumeration, which could then
   cause a `set_meta_spool has not been called` panic.
 * HTTP Injection and XFER Injections didn't grab an Activity handle which
   meant that there was a potential race condition when shutting down the
   system which could result in loss of accountability of the message(s)
   that were part of that request.
 * Fixed possible integer overflow when computing a very long delay.
   Thanks to @edgarsendernet! #480
 * Filter out not-relevant-to-TSA records earlier in the logging pipeline. #478
 * Outbound SMTP connections that have been closed by the destination during
   idle time are now detected more robustly in between message sends, reducing
   the rate at which a message will get classified as an internal connection
   failure. #482
