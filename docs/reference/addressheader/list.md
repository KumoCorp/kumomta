# list

```lua
local list = addressheader.list
```

Returns a lua table consisting of one entry per address in the header. Each
entry is a single simple address object that has `domain`, `user`, `email` and
`name` fields with the same semantics as `addressheader`.

```lua
for _, address in ipairs(msg:to_header().list) do
  print('to entry', address)
  -- prints something like:
  -- to entry      {"name":null,"address":"user@example.com"}
  -- to entry      {"name":"John Smith","address":"john.smith@example.com"}
end
```
