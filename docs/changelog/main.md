# Unreleased Changes in The Mainline

## Breaking Changes

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

## Fixes

 * sources helper didn't allow creating empty egress pools
