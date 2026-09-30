# lookup_addr

```lua
kumo.dns.lookup_addr(NAME, OPT_RESOLVER_NAME, OPT_STRATEGY)
```

{{since('2023.08.22-4d895015')}}

Resolve the `A` and `AAAA` records for the requested `NAME`.

Returns an array style table listing the IPv4 and IPv6 addresses as strings.
An ordinary no-address result (NODATA or NXDOMAIN) returns an empty table.

Bogus DNSSEC answers and failure responses such as `SERVFAIL` are rejected,
not interpreted as unsigned addresses or an ordinary empty result. Depending
on the lookup strategy, addresses from another successful family may still be
returned. An error is raised if no addresses are available and a lookup failed.

With the default resolver, address results are cached according to their DNS
TTL. DNS errors and bogus answers are not stored in Kumo's address caches as
empty or unsigned results. If one queried address family fails, the incomplete
combined result is not cached, but successful per-family answers retain their
normal TTLs. A later lookup can retry the failed family without waiting for those
TTLs to expire. Specifying `OPT_RESOLVER_NAME` bypasses these caches; resolver
backends may also maintain their own DNS caches.

```lua
print(kumo.json_encode(kumo.dns.lookup_addr 'localhost'))

-- prints out:
-- ["127.0.0.1","::1"]
```

{{since('2025.12.02-67ee9e96')}}

The `OPT_RESOLVER_NAME` parameter is an optional string parameter that
specifies the name of a alternate resolver defined via
[define_resolver](define_resolver.md).  You can omit this parameter and the
default resolver will be used.

{{since('2026.04.09-ea3b2a9b')}}

The `OPT_STRATEGY` parameter is an optional string parameter that specifies the
IPv4 vs. IPv6 lookup strategy.  Allowable values and default behavior (if you
omit this parameter) are the same as those described in
[ip_lookup_strategy](../kumo/make_egress_path/ip_lookup_strategy.md).

