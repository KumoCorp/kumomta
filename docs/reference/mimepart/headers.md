# headers

```lua
local headers = mimepart.headers
```

!!! note
    This is a field rather than a method, so you must use `mimepart.headers`
    rather than `mimepart:headers` to reference it.

{{since('dev')}}

The `headers` field returns a reference to the
[HeaderMap](../headermap/index.md) for `mimepart`.  You can use the headermap
to enumerate or modify the set of headers in the mime part.

The example below prints the `Subject` header of the mime part:

```lua
local subject = mimepart.headers.subject()
print(subject)
```

