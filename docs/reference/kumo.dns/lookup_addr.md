# lookup_addr

```lua
kumo.dns.lookup_addr(NAME, OPT_RESOLVER_NAME)
```

{{since('2023.08.22-4d895015')}}

Resolve the `A` and `AAAA` records for the requested `NAME`.

Raises an error if the name doesn't exist in DNS.

Returns an array style table listing the IPv4 and IPv6 addresses as strings.

DNS results are cached according to the TTL specified by the DNS record itself.

```lua
print(kumo.json_encode(kumo.dns.lookup_addr 'localhost'))

-- prints out:
-- ["127.0.0.1","::1"]
```

{{since('2025.12.02-67ee9e96')}}

The `OPT_RESOLVER_NAME` is an optional string parameter that specifies the name
of a alternate resolver defined via [define_resolver](define_resolver.md).  You
can omit this parameter and the default resolver will be used.
