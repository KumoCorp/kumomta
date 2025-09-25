# message_id

```lua
local message_id = headers:message_id()
```

{{since('dev')}}

Parses the `Message-Id` header, and if present, returns the id string.
Returns `nil` if `Message-Id` is not present.

