# shrink

```lua
message:shrink()
```

{{since('2025.03.19-1d3f1f67')}}

This method will ensure that the message contents are journalled to the spool,
and then release any in-memory body and metadata information.

See also:
* [msg:shrink_data()](shrink_data.md)
