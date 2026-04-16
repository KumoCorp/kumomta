# get_first_named_header_value

```lua
message:get_first_named_header_value(NAME)
```

Gets the first header whose name matches `NAME`, decode it to UTF-8 and return
it.

Returns `nil` if no matching header could be found.

{{since('dev')}}

When the structured header parser fails (for example, due to non-conforming
header content), this method now falls back to returning the raw header value
rather than raising an error.  This improves resilience when processing
messages with headers that do not strictly conform to the RFCs.
The same fallback applies to `get_all_named_header_values` and `get_all_headers`.


