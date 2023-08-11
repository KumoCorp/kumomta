# `kumo.dns.lookup_addr(NAME)`

{{since('dev')}}

Resolve the `A` and `AAAA` records for the requested `NAME`.

Raises an error if the name doesn't exist in DNS.

Returns an array style table listing the IPv4 and IPv6 addresses as strings.

DNS results are cached according to the TTL specified by the DNS record itself.

```lua
print(kumo.json_encode(kumo.dns.lookup_addr 'localhost'))

-- prints out:
-- ["127.0.0.1","::1"]
```
