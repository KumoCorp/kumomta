# Configuring Spooling

KumoMTA supports a number of message spooling options. A spool must be defined, but setting deferred spooling in the listener will force the system to attempt delivery once without hitting the spool first.  Deferred messages will then be spooled for later attempts.  

There are two kinds of spool storage possible.  The default is in files on the local disk, defined as `kind=LocalDisk`.  The alternate is to use RocksDB (defined as `kind=RocksDB`) for a higher throughput.  This is less conventional, but has proven to yield higher performance.   Below is a sample of configuring a RocksDB spool.

```lua
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = `RocksDB`,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = `RocksDB`,
  }
```