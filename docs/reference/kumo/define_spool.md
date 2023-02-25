# `kumo.define_spool {PARAMS}`

Defines a named spool storage backend.

KumoMTA uses separate storage areas for metadata and message contents, named
`"meta"` and `"data"` respectively.

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## flush

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

## kind

Specifies the spool storage backend type. There are two possible options:

* `"LocalDisk"` - the default. Stores data in individual files on the filesystem.
* `"RocksDB"` - uses [RocksDB](https://rocksdb.org/) to achieve higher throughput.

`"LocalDisk"`'s performance characteristics are strongly coupled with your
local storage device and filesystem performance.

`"RocksDB"` makes heavy use of memory buffers and intelligent layout of storage
to reduce the I/O cost. To a certain degree, the buffering has similar
characteristics to deferred spooling, but the risk of corruption is attenuated
because RocksDB uses a write-ahead-log and a background sync thread.

## name

Specify the name of this spool. You are free to define as many spools as
you wish, but at the time of writing KumoMTA only uses spools with the following names:

* `"data"` - holds the message bodies
* `"meta"` - holds message metadata, such as the envelope and per message metadata

## path

Specifies the path to the directory into which the spool will be stored.

```lua
kumo.on('init', function()
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo-spool/data',
  }
end)
```
