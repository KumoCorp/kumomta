# message_id

```lua
local message_id = header.message_id
```

{{since('dev')}}

Reading the `message_id` field will attempt to interpret the contents
of the header as a `Message-Id` header.

If the header value is not compatible with this representation, a lua error
will be raised.
