# ptr_host

```lua
local domain = kumo.dns.ptr_host(IP)
```

{{since('dev')}}

Given an IP address in either V4 or V6 format as the input, returns
the reversed address plus appropriate top level domain suitable for
performing a reverse lookup.

This function is purely local string manipulation; no actual DNS queries are
performed.

You will typically use [lookup_ptr](lookup_ptr.md) to perform an actual PTR
lookup; this function is a utility function for the cases where you're doing
something unusual.

```lua
print(kumo.dns.ptr_host '127.0.0.1')
-- prints out:
-- 1.0.0.127.in-addr.arpa
```

```lua
print(kumo.dns.ptr_host '::1')
-- prints out:
-- 1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa
```
