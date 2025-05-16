# shrink_data

```lua
message:shrink_data()
```

{{since('2025.03.19-1d3f1f67')}}

This method will ensure that the message contents are journalled to the spool,
and then release any in-memory body data, keeping the metadata in-memory.

See also:
* [msg:shrink()](shrink.md)

