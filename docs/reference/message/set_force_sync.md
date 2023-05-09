# `message:set_force_sync(force)`

When `force` is `true`, each future attempt to save the message metadata or
data will use a high durability write, delaying further processing until the
message data has been written to the spool.

When `force` is `false`, which is the default setting, whether high durability
writes are used is a function of the configuration of the spool(s) that you
have enabled in your configuration.

!!! note
   Using this together with RocksDB spool can be incredibly harmful to overall
   performance, as forcing a flush is a database-wide operation.
