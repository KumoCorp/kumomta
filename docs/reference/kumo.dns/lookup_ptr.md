# lookup_ptr

```lua
kumo.dns.lookup_ptr(IP)
```

{{since('dev')}}

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
