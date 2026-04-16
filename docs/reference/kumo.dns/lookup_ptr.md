# lookup_ptr

```lua
kumo.dns.lookup_ptr(IP, OPT_RESOLVER_NAME)
```

{{since('2025.10.06-5ec871ab')}}

Resolve PTR records for the requested `IP`.

Raises an error if there was an issue resolving the record.

Returns a lua array-style table with the list of PTR records returned from DNS.  The table may be empty.

```lua
local ok, records = pcall(kumo.dns.lookup_ptr, '127.0.0.1')
if ok then
  for _, a in ipairs(records) do
    print(a)
  end
end
```

{{since('2025.12.02-67ee9e96')}}

The `OPT_RESOLVER_NAME` is an optional string parameter that specifies the name
of a alternate resolver defined via [define_resolver](define_resolver.md).  You
can omit this parameter and the default resolver will be used.
