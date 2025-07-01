# lookup_ptr

```lua
kumo.dns.lookup_ptr(IP)
```

Resolve PTR records for the requested `IP`.

Raises an error if there was an issue resolving the record.

Returns a lua array-style table with the list of A records returned from DNS.

```lua
local ok, records = pcall(kumo.dns.lookup_ptr, '127.0.0.1')
if ok then
  for _, a in pairs(records) do
    print(a)
  end
end
```
