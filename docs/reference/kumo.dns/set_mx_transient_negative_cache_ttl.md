# set_mx_transient_negative_cache_ttl

```lua
kumo.dns.set_mx_transient_negative_cache_ttl(DURATION)
```

{{since('dev')}}

Set the negative cache TTL used when an MX resolution fails without an
authoritative answer: a query timeout, a SERVFAIL or other failure response, or
a resolver I/O error. Such a failure may clitear on its own. This TTL should be kept
short, well below [set_mx_negative_cache_ttl](set_mx_negative_cache_ttl.md),
to retry the lookup sooner than the other classes of error.

This is distinct from
[set_mx_negative_cache_ttl](set_mx_negative_cache_ttl.md), which governs how
long an authoritative NXDOMAIN is cached.

An MX lookup that timed out before the DNS query could even be issued, because
the MX concurrency limit was saturated, is not cached at all. The next lookup
retries immediately regardless of either TTL.

`DURATION` is either a number expressed as optionally fractional seconds,
or a human readable duration string like `"5s"` to specify the units.

The default value for this is `"30 seconds"`.

```lua
kumo.on('pre_init', function()
  kumo.dns.set_mx_transient_negative_cache_ttl '1m'
end)
```
