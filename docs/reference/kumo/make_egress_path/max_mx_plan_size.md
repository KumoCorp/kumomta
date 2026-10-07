# max_mx_plan_size

{{since('dev')}}

The largest number of candidate addresses to retain in the connection plan
that is built when delivering to a site.

When resolving a destination, each of its MX hosts is expanded to every one of
its `A`/`AAAA` addresses. `max_mx_plan_size` caps the total number of addresses
kept across all of those hosts to prevent a destination publishing a very large
number of addresses from forcing the system to build and retain an unbounded
plan, or to spend an unbounded amount of time working through it when
connections fail.

Addresses are collected in MX preference order, most-preferred first, and the
cap is applied by truncating that list. The addresses dropped once the limit is
reached are from the least-preferred hosts. The per-host cap
[max_mx_addresses_per_host](max_mx_addresses_per_host.md) is applied first.

The default value is `50`.
