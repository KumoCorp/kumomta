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

## Other Changes and Enhancements

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

## Fixes

 * sources helper didn't allow creating empty egress pools
 * RFC5965 and RFC3464 parsing now strips enclosing angle brackets from envelope
   address fields in the ARF/OOB message.
 * smtp server: invalid addresses passed to MAIL FROM or RCPT TO would result
   in a 421 response instead of the more appropriate 501 permanent failure
   response. #495
 * smtp server: uncommon quoted local parts containing the `@` sign are now
   accepted by the envelope address parser. #495
