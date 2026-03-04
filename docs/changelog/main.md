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

## Fixes

