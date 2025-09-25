# message_id_list

```lua
local message_id_list = header.message_id_list
```

{{since('dev')}}

Reading the `message_id_list` field will attempt to interpret the contents of the
header as list of `Message-Id` header values.

If the header value is not compatible with this representation, a lua error
will be raised.
