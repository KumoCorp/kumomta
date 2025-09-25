# mime_version

```lua
local mime_version = headers:mime_version()
```

{{since('dev')}}

Parses the `Mime-Version` header, and if present, returns it as a string.
Returns `nil` if `Mime-Version` is not present.

