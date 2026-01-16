# Unreleased Changes in The Mainline

## Breaking Changes

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
 * The [HTTP Injection API](../reference/http/api_inject_v1.md) and [MIME
   Builder API](../reference/kumo.mimepart/builder.md) now support creating
   messages with [AMP
   HTML](https://amp.dev/documentation/guides-and-tutorials/email/learn/email-spec/amp-email-structure)
   parts.
   [mimepart:get_simple_structure()](../reference/mimepart/get_simple_structure.md)
   also supports AMP HTML parts.
 * Improved the context shown in error messages produced by the HTTP injection
   API

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
