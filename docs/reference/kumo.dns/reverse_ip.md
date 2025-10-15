# reverse_ip

```lua
local reversed = kumo.dns.reverse_ip(IP)
```

{{since('dev')}}

Given an IP address in either V4 or V6 format as the input, returns
the reversed address.

This function is purely local string manipulation; no actual DNS queries are
performed.

```lua
print(kumo.dns.reverse_ip '127.0.0.1')
-- prints out:
-- 1.0.0.127
```

```lua
print(kumo.dns.reverse_ip '::1')
-- prints out:
-- 1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0
```

