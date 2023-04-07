# Configuring Spooling

KumoMTA uses separate storage areas for metadata and message contents, named *meta* and *data* respectively. The spool is defined as part of the init event within the server's init.lua policy.

KumoMTA supports multiple message spooling options.

There are two kinds of spool storage possible:

* LocalDisk writes to the specified path on disk, separating messages from their metadata. LocalDisk will have a heavy performance dependency on your filesystem IO performance, and it is strongly recommended that the spool be mounted on separate storage from the logs and the rest of the server OS for maximum performance. If SSD drives are not used, 15K RPM disks are recommended. When using disk spooling, we recommend using ext4 with the *noatime* flag.

```text
LABEL=/var/spool/kumomta/data /var/spool/kumomta/data ext4 rw,noatime,barrier=0 0 2
```

LocalDisk is the default, so it does not need to be explicitly specified:

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

For additional performance, you can configure your listeners to defer spooling on their messages, see the [Configuring SMTP Listeners](./smtplisteners.md) page for more information.

* A higher performance option is RocksDB (defined as *kind=RocksDB).*

RocksDB makes heavy use of memory buffers and intelligent layout of storage
to reduce I/O cost and increase performance. This gives increased performance similar to deferred spooling but with less risk because RocksDB uses a write-ahead-log and a background sync thread.

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
