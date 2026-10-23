# kind

Specifies the spool storage backend type. This field is required. There is no
default. The possible options are:

* `"LocalDisk"` - stores data in individual files on the filesystem.
* `"RocksDB"` - uses [RocksDB](https://rocksdb.org/) to achieve higher throughput.

!!! warning
    The `"LocalDisk"` kind is deprecated and will be removed in a future
    release. Defining a spool with it will log an error. Use `"RocksDB"` instead.
    Earlier versions allowed omitting `kind` and assumed that you meant `LocalDisk`.
    {{since('dev', inline=True)}}

`"LocalDisk"`'s performance characteristics are strongly coupled with your
local storage device and filesystem performance.

`"RocksDB"` makes heavy use of memory buffers and intelligent layout of storage
to reduce the I/O cost. To a certain degree, the buffering has similar
characteristics to deferred spooling, but the risk of corruption is attenuated
because RocksDB uses a write-ahead-log and a background sync thread.

