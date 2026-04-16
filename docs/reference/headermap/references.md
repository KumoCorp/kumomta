# references

```lua
local references = headers:references()
```

{{since('2025.10.06-5ec871ab')}}

Parses the `References` header, if present, returning an array style table
(list) of `Message-Id` strings to which this message refers.

Returns `nil` if no `References` header is present.
