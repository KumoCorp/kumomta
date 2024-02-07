# Unreleased Changes in The Mainline

## Breaking Changes

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

