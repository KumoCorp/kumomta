# kumo.configure_suppression_store

```lua
kumo.configure_suppression_store { PARAMS }
```

Configure the storage backend for the suppression list.

This function should be called only from inside your [init](../events/init.md)
event handler.

If not called, the suppression list will use in-memory storage by default,
which means data is lost when the server restarts. For production deployments,
RocksDB storage is recommended to ensure suppression list data persists across
restarts.

*PARAMS* is an object-style table that accepts the following keys:

* `kind` - required string. The type of storage backend to use:
    * `"in_memory"` - Fast in-memory storage. Data is lost when the server restarts.
    * `"rocks_db"` - Persistent storage using RocksDB. Data survives server restarts.

When `kind = "rocks_db"`, the following additional keys are available:

* `path` - required string. The filesystem path where the RocksDB database
will be stored. The directory will be created if it does not exist.
* `flush` - optional boolean. Whether to enable fsync for durability. When
`true`, data is synced to disk on each write, ensuring durability at the
cost of performance. Default: `true`.
* `compression` - optional string. The compression algorithm to use for stored
data:
    * `"none"` - No compression
    * `"snappy"` - Fast compression with moderate ratio
    * `"lz4"` - Good balance of speed and compression (default)
    * `"zstd"` - Best compression ratio, slightly slower

!!! note
    This function can only be called once. Calling it again will result in an error.

## Examples

### In-Memory Storage (Default)

```lua
kumo.on('init', function()
kumo.configure_suppression_store {
    kind = 'in_memory',
}
end)
```

### Persistent RocksDB Storage

```lua
kumo.on('init', function()
kumo.configure_suppression_store {
    kind = 'rocks_db',
    path = '/var/spool/kumomta/suppression',
    flush = true,
    compression = 'lz4',
}
end)
```

### High-Performance Setup

For maximum write performance (at the cost of potential data loss on crash):

```lua
kumo.on('init', function()
kumo.configure_suppression_store {
    kind = 'rocks_db',
    path = '/var/spool/kumomta/suppression',
    flush = false,
    compression = 'none',
}
end)
```

## See Also

* [Suppression List API](../http/api_admin_suppression_v1.md)
* [kumo.api.admin.suppression module](../kumo.api.admin.suppression/index.md)
