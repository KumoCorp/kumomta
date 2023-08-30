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

## Fixes
* HTTP injection API did not expand templating in `From`, `Reply-To` or
  `Subject` headers unless they were set in the additional headers object
