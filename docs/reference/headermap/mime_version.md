# mime_version

```lua
local mime_version = headers:mime_version()
```

{{since('2025.10.06-5ec871ab')}}

Parses the `MIME-Version` header, and if present, returns it as a string.
Returns `nil` if `MIME-Version` is not present.

