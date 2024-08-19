# name

Required string.

The name of the source.

```lua
kumo.on('get_egress_source', function(source_name)
  -- Make a source that just has the requested name, but otherwise doesn't
  -- specify any particular source configuration
  return kumo.make_egress_source {
    name = source_name,
  }
end)
```


