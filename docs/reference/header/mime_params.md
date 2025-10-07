# mime_params

```lua
local params = header.mime_params
```

{{since('2025.10.06-5ec871ab')}}

Reading the `mime_params` field will attempt to interpret the contents of the
header as a [MimeParams](../headermap/index.md#mimeparams).

If the header value is not compatible with this representation, a lua error
will be raised.
