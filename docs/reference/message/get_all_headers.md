# get_all_headers

```lua
message:get_all_headers()
```

Gets the all the headers, decode the values to UTF-8 and
return them in a lua array style table of tables:

```lua
local headers = message:get_all_headers()
assert(headers == {
  { 'Subject', 'The Subject' },
  { 'Date', 'Sun Feb 26 02:45:02 PM MST 2023' },
})
```

