# Unreleased Changes in The Mainline

## Breaking Changes
 * The `rfc5321_rustls_config` cache has been renamed to `rustls_client_config`.
   If you have a policy that tunes this cache via
   [kumo.set_lruttl_cache_capacity](../reference/kumo/set_lruttl_cache_capacity.md),
   you will need to update the cache name.

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
     * Use [kumo.start_proxy_listener](../reference/kumo/start_proxy_listener/index.md)
       function to configure a SOCKS5 proxy server
     * Many thanks to @vietcgi! #459
 * New [kumo.xfer.xfer](../reference/kumo.xfer/xfer.md) function to enable
   per-message transfer between nodes, which is useful in combination with the
   [requeue_message](../reference/events/requeue_message.md) event.

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
