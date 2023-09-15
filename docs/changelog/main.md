# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
* New RFC-conformance options are available to control server behavior
  when receiving messages that are non-conformant:
     * [invalid_line_endings](../reference/kumo/start_esmtp_listener.md#invalid_line_endings) #22 #23
     * [line_length_hard_limit](../reference/kumo/start_esmtp_listener.md#line_length_hard_limit) #25
     * [message:check_fix_conformance](../reference/message/check_fix_conformance.md) #17 #24 #26
* HTTP injection API will now parse and re-encode the message content to ensure
  that it has appropriate transfer encoding applied when `content` is set to a
  string, rather than using the builder variant of the API.
* Preliminary support for
  [MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461). It defaults to
  enabled. See
  [make_egress_path](../reference/kumo/make_egress_path.md#enable_mta_sts) for
  more details. At this time, we do not support
  [TLSRPT](https://datatracker.ietf.org/doc/html/rfc8460).
* The [DKIM
  helper](../userguide/configuration/dkim.md#using-the-dkim_signlua-policy-helper)
  now allows setting `body_canonicaliation` and `header_canonicalization`.
  Thanks to @cai-n! #81
* [msg:dkim_verify()](../reference/message/dkim_verify.md) and
  [msg:add_authentication_results()](../reference/message/add_authentication_results.md)
  #82
* You may now use `log_record` to refer to the entire log record when using
  templated log files. See
  [kumo.configure_local_logs](../reference/kumo/configure_local_logs.md) for
  more information.
* You may now specify an optional `segment_header` when customizing local logs.
  See [kumo.configure_local_logs](../reference/kumo/configure_local_logs.md)
  for more information.

## Fixes
* HTTP injection API did not expand templating in `From`, `Reply-To` or
  `Subject` headers unless they were set in the additional headers object
* Allow optional spaces after the colon in `MAIL FROM:` and `RCPT TO:`. #76
* Missing 334 response to clients using multi-step SMTP `AUTH PLAIN`
