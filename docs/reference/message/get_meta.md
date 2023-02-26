# `message:get_meta(KEY)`

Messages are associated with some metadata. You can think of this metadata
as being equivalent to a JSON object.

The `get_meta` method allows you to retrieve a field of that object.

```lua
msg:set_meta('foo', 'bar')
print(msg:get_meta 'foo') -- prints 'bar'
```

See also [msg:set_meta](set_meta.md).
