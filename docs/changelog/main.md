# Unreleased Changes in The Mainline

## Breaking Changes
 * DKIM verification no longer implicitly generates a policy failure if
   the From: domain doesn't match the DKIM signature.  It was an overly
   restrictive non-standard check that was carried over from when we
   imported that DKIM dependency.  If you require such a policy, you can
   iterate over the authentication results that are returned by 
   `msg:dkim_verify()` and check the signatures and compare their
   domains against the message and take whatever action is appropriate
   to your policy.
 * [address.user](../reference/address/user.md) now returns the *normalized
   local part* rather than the raw local part.  This affects the uncommon
   quoted local part form of an address, such as `"quoted"@example.com`.
   This behavior also applies to the local part values that are used
   to construct [Advanced Maildir
   Paths](../reference/kumo/make_queue_config/protocol.md#advanced-maildir-path).
 * Our MIME parser now accommodates non-conforming binary message
   content that is neither ASCII nor UTF-8, such as message content encoded in
   `shift-jis` without using appropriate MIME markup to indicate that encoding.
   Such content is technically not relayable according to the SMTP RFCs but
   is commonly produced and accepted in Japan.  For the most part this
   change is transparent and has no downsides but it can result in various
   methods on the `Message` and `MimePart` types returning binary strings
   to lua where they were formerly guaranteed to be UTF-8.  Be aware that
   [msg:check_fix_conformance](../reference/message/check_fix_conformance.md)
   can be used with charset detection enabled to rewrite such a message
   into conforming MIME.  If you need to relay this sort of content it is often
   undesirable to rewrite it so you will need to take appropriate care in your
   policy to decide when to preserve, fix or reject this content.
 * When all connection attempts fail due to unplumbed IPs (either local, or via
   kumo-proxy), or are due to failure to connect to a proxy, we now summarize
   this and the resulting transient failure will include a string like `All
   failures are related to the proxy server having an unplumbed source
   address`, `All failures are related to proxy connection issues` or `All
   failures are related to having an unplumbed source address`.  If you
   have automation rules that depend on the precise formatting of these
   sorts of failures, you should review and revise them accordingly.
   Note that it is not possible to detect unplumbed IPs via HA Proxy
   due to limitations of the HA Proxy protocol.

## Other Changes and Enhancements

 * New [kumo.jsonl](../reference/kumo.jsonl/index.md) module providing
   utilities for reading and writing zstd-compressed JSONL log segment files.
   This is useful when you want to implement webhook delivery (or other
   log-driven processing) **out of band** from the in-process
   [log hook](../userguide/operation/webhooks.md) mechanism — for example,
   to run a separate long-lived script that reads from the on-disk logs and
   forwards them to an HTTP endpoint with independent checkpointing and retry
   semantics.  See the
   [Batched Webhook Example](../reference/kumo.jsonl/new_tailer.md#batched-webhook-example)
   and the
   [Per-Customer Webhook Example](../reference/kumo.jsonl/new_tailer.md#per-customer-webhook-example-with-main-parameters)
   in the `kumo.jsonl.new_tailer` docs for worked examples.
 * queue helper: certain misconfigurations are now detected at startup,
   improving error discovery.
 * New
   [ip_lookup_strategy](../reference/kumo/make_egress_path/ip_lookup_strategy.md)
   option controlling how `A`/`AAAA` lookups are performed when
   resolving MX hosts.  Since this option is set in the egress path, it means
   that you can control resolution on a per-source basis if you wish.

 * memoize and the lruttl cache layer will now consider a pending populate
   that has taken longer than the populate timeout to have been abandoned,
   once a subsequent lookup is initiated.  This may cause pre-existing waiters to
   awake and report the cache lookup as failed, but will unblock future
   lookups.  In addition, we now bound the number of retries in this sort
   of internal inconsistency state to 10, which may cause errors to be
   reported earlier and/or more frequently than in prior versions, but should
   result in less of an overall bottleneck in the triggering scenario.

 * [kumo.serde.json_encode_pretty](../reference/kumo.serde/json_encode_pretty.md)
   now outputs keys of json objects in sorted order.  This means that utilities
   such as `resolve-shaping-domain` will now output keys in sorted order as well.

 * [kumo.encode.charset_encode](../reference/kumo.encode/charset_encode.md) and
   [kumo.encode.charset_decode](../reference/kumo.encode/charset_decode.md) string
   charset encoding/decoding functions for advanced use cases.

 * [kumo.string.starts_with](../reference/string/starts_with.md) and
   [kumo.string.ends_with](../reference/string/ends_with.md). Thanks to
   @kayozaki! #498

 * New [kumo.fs.metadata_for_path](../reference/kumo.fs/metadata_for_path.md) and
   [kumo.fs.symlink_metadata_for_path](../reference/kumo.fs/symlink_metadata_for_path.md) functions to get file/directory
   metadata. Thanks to @kayozaki! #497

 * Queue Helper: `queue_module:setup_with_options` has a new optional
   `invalidate_with_epoch` boolean parameter that will set the underlying
   `queue_helper_data` cache to be invalidated when the [config
   epoch](../reference/configuration.md#config-epoch) is bumped.  This option
   allows you to achieve lower latency configuration updates in exchange for
   higher CPU overhead around the time of the configuration update.

## Fixes

 * sources helper didn't allow creating empty egress pools
 * RFC5965 and RFC3464 parsing now strips enclosing angle brackets from envelope
   address fields in the ARF/OOB message.
 * smtp server: invalid addresses passed to MAIL FROM or RCPT TO would result
   in a 421 response instead of the more appropriate 501 permanent failure
   response. #495
 * smtp server: uncommon quoted local parts containing the `@` sign are now
   accepted by the envelope address parser. #495
 * smtp client: when using client certificates with openssl, we did not load
   or propagate intermediate certificates from the supplied certificate data,
   which could lead to the peer failing to verify the client certificate.
   Thanks to @kayozaki! #496
 * HTTP injection API: a `to_header` template substitution is now
   pre-defined with the default `To` header value that would be generated
   for the recipient. You can use `{{ to_header }}` in your `To` header
   template and optionally override `to_header` in the per-recipient
   substitutions for recipients where you want a different value.
   Thanks to @Harshjha3006! #501
