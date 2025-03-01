# flush

Whether to flush data to storage after each write. The default is `false`.
When set to `true`, a backend specific means of flushing OS buffers to storage
will be used after each write to encourage maximum durability of writes.

Setting `flush=true` can be incredibly harmful to throughput, and, depending
on your local storage device and filesystem selection, may not meaningfully
increase durability.

```lua
kumo.on('init', function()
  kumo.define_spool {
    -- ..
    flush = false,
  }
end)
```


