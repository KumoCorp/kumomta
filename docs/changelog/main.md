# Unreleased Changes in The Mainline

## Breaking Changes
* We now normalize line breaks in SMTP client responses into spaces, to make it
  a little easier to deal with multi-line responses in both TSA automation,
  response rewriting and bounce classification. Thanks to @cai-n! #157
* We now also strip the enhanced status code prefix from each subsequent line
  of multi-line responses, rather than just the first line. This may influence
  your classification or TSA automation regexes.  For example, for a response
  like `500-5.5.5 foo\n500 5.5.5 bar` we would previously represent that
  with a logged `response.content` value of `foo\n5.5.5 bar`, but will now
  encode it as `foo bar`. See #157 for more context.

## Other Changes and Enhancements

* Added a default timeout of 60 seconds to the HTTP client returned from
  [kumo.http.build_client](../reference/kumo.http/build_client.md).
  Added [request:timeout()](../reference/kumo.http/Request.md#requestimeout)
  method to explicitly set a timeout value.
* You may now list multiple `regex`s and/or multiple `action`s for TSA rules
  by using an array in your toml file. Single values are still supported. #99
* Added [over_sign](../reference/kumo.dkim/rsa_sha256_signer.md#over_sign)
  optional to easily enabled DKIM over-signing to protect your messages
  against replay attacks. The same option can be set for the ed25519_signer
  as well. #111
* Updated RocksDB from 8.1 to 8.10
* Slightly relaxed the MIME parser so that we can tolerate non-conforming 8-bit
  message bodies with missing transfer encoding when all we need is to parse
  headers. The invalid messages will trigger `NEEDS_TRANSFER_ENCODING` when
  using
  [msg:check_fix_conformance()](../reference/message/check_fix_conformance.md)
  to validate messages, but won't cause header parsing to fail in general.
  These non-compliant messages will be parsed (or fixed) using a lossy decode
  to UTF-8 that will remap invalid bytes to `U+FFFD` the Unicode Replacement
  Character.
* Added `max_message_rate` option to
  [kumo:make_queue_config](../reference/kumo/make_queue_config.md)
  to configure the rate at which a given Scheduled Queue can feed
  into the Ready Queue.
* Added
  [request_body_limit](../reference/kumo/start_http_listener.md#request_body_limit)
  option to raise the default HTTP request size, which is useful when
  performing HTTP based injection with large message payloads.
* It is now possible to use `protocol` in the `queues.toml` lua helper
  configuration file. Thanks to @aryeh! #155
* The TSA `Suspend` action will now generate suspensions that are visible
  via the HTTP API and kcli utility, and that will take effect in realtime.
* TSA now supports `SuspendTenant` and `SuspendCampaign` actions that allow
  reacting to source-domain-specific tempfails. These will also be visible
  via the HTTP API and kcli utility, and also take effect in realtime.
* New [glob](../reference/kumo/glob.md),
  [read_dir](../reference/kumo/read_dir.md) and
  [uncached_glob](../reference/kumo/uncached_glob.md) filesystem functions.
  #161
* New [kumo.api.inject.inject_v1](../reference/kumo.api.inject/inject_v1.md) lua
  function for constructing and injecting arbitrary messages via policy. #102

## Fixes

* The `delivered_this_connection` counter was incorrectly double-counted for
  SMTP sessions, effectively halving the effective value of
  `max_deliveries_per_connection`.
* Re-run the ready queue maintainer immediately after closing a connection
  due to reaching the `max_deliveries_per_connection`, so that new connection(s)
  can be established to replace the one that just closed. Previously, we would
  only do this once every minute. #116
* The `smtp_client_rewrite_delivery_status` event could trigger with incorrect
  scheduled queue name components.
* webhooks and other lua delivery handlers didn't reuse connections correctly.
  Thanks to @cai-n! #135
* `OOB` and `ARF` reports were incorrectly logged as `Reception` records
* `OOB` reports did not respect `headers` and `meta` configured in the logger
* MIME Parser would discard whitespace from improperly encoded `message/rfc822`
  parts when rebuilding messages.
* proxy-server didn't actually bind to the requested source address
* `listener_domains.lua` helper didn't always fallback to full wildcard/default
  (`*`) entries correctly. #128
* smtp client did not always wait for the full extent of the idle timeout for
  new messages before closing the connection.
