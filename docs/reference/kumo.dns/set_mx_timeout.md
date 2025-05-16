# set_mx_timeout

```lua
kumo.dns.set_mx_timeout(DURATION)
```

{{since('2025.03.19-1d3f1f67')}}

Set overall time limit for MX record resolution.  This applies to the MX record
resolution, rather than the resolution of MX hosts into addresses.

`DURATION` is either a number expressed as optionally fractional seconds,
or a human readable duration string like `"5s"` to specify the units.

The default value for this is `"5 seconds"`.

!!! note
    This timeout is layered over any internal timeout that the underlying DNS
    resolver might be configured to use, which means that the effective timeout
    will be the smallest of both this value and what is configured for the
    underlying resolver.

```lua
kumo.on('pre_init', function()
  kumo.dns.set_mx_timeout(5)
  -- or alternatively: kumo.dns.set_mx_timeout("5s")
end)
```
