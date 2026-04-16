# message_id

```lua
local message_id = headers:message_id()
```

{{since('2025.10.06-5ec871ab')}}

Parses the `Message-Id` header, and if present, returns the id string.
Returns `nil` if `Message-Id` is not present.

