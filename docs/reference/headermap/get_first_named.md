# get_first_named

```lua
headers:get_first_named(NAME)
```

{{since('2025.10.06-5ec871ab')}}

Gets the first header whose name equals `NAME` (case insensitive) and return
the [Header](../header/index.md) object for that header.

Returns `nil` if no matching header could be found.

