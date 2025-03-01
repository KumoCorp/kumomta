# kind

Specifies the spool storage backend type. There are two possible options:

* `"LocalDisk"` - the default. Stores data in individual files on the filesystem.
* `"RocksDB"` - uses [RocksDB](https://rocksdb.org/) to achieve higher throughput.

`"LocalDisk"`'s performance characteristics are strongly coupled with your
local storage device and filesystem performance.

`"RocksDB"` makes heavy use of memory buffers and intelligent layout of storage
to reduce the I/O cost. To a certain degree, the buffering has similar
characteristics to deferred spooling, but the risk of corruption is attenuated
because RocksDB uses a write-ahead-log and a background sync thread.

