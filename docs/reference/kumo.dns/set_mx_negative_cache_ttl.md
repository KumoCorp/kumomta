# set_mx_negative_cache_ttl

```lua
kumo.dns.set_mx_negative_cache_ttl(DURATION)
```

{{since('2025.03.19-1d3f1f67')}}

Set the negative cache TTL that should be used when caching an MX resolution
failure.

`DURATION` is either a number expressed as optionally fractional seconds,
or a human readable duration string like `"5s"` to specify the units.

The default value for this is `"5 minutes"`.

```lua
kumo.on('pre_init', function()
  kumo.dns.set_mx_negative_cache_ttl '10m'
end)
```

