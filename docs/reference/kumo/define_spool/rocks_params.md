# rocks_params

Specifies additional tuning parameters for RocksDB when `kind = "RocksDB"`.

The following parameters are possible:

## increase_parallelism

{{since('2023.03.31-36aa20de')}}

By default, RocksDB uses only one background thread for flush and compaction.
You can use this option to increase the number of threads available for this purpose.
A good number is the number of cores available to the system:

```lua
kumo.on('init', function()
  local params = {
    increase_parallelism = kumo.available_parallelism(),
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## optimize_level_style_compaction

{{since('2023.03.31-36aa20de')}}

When set, its value is the number of bytes to use for the
`memtable_memory_budget` and enable the use of level-style compaction.

Level style compaction is the default.

Larger values allocate more memory to use for write and compaction buffers
and can increase throughput to RocksDB.

This option is incompatible with `optimize_universal_style_compaction`.

```lua
kumo.on('init', function()
  local params = {
    -- Use 64MB as the base write_buffer size
    optimize_level_style_compaction = 64 * 1024 * 1024,
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## optimize_universal_style_compaction

{{since('2023.03.31-36aa20de')}}

When set, its value is the number of bytes to use for the
`memtable_memory_budget` and enable the use of universal-style compaction.

Larger values allocate more memory to use for write and compaction buffers
and can increase throughput to RocksDB.

This option is incompatible with `optimize_level_style_compaction`.

```lua
kumo.on('init', function()
  local params = {
    -- Use 64MB as the base write_buffer size
    optimize_universal_style_compaction = 64 * 1024 * 1024,
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## paranoid_checks

{{since('2023.03.31-36aa20de')}}

When `true`, RocksDB performs additional consistency checks during
open and on every read/write operation, including block-level
checksum verification.  This catches more on-disk corruption at the
cost of additional CPU.  Defaults to `false`.

## compression_type

{{since('2023.03.31-36aa20de')}}

Controls the compression algorithm applied to SST blocks.

The official kumod builds link a statically-built RocksDB that
includes only the `"None"` and `"Snappy"` (the default) backends.
Other values are accepted by the configuration parser but will
fail at runtime unless you are running a custom build of kumod
that links a RocksDB compiled with the corresponding compression
backend enabled:

| Value | Available in official builds? |
|---|---|
| `"None"` | yes |
| `"Snappy"` (default) | yes |
| `"Zlib"` | no -- requires a custom build |
| `"Bz2"` | no -- requires a custom build |
| `"Lz4"` | no -- requires a custom build |
| `"Lz4hc"` | no -- requires a custom build |
| `"Zstd"` | no -- requires a custom build |

Snappy offers a good balance of CPU cost and compression ratio
for typical message payloads.  `"None"` is appropriate when the
underlying storage is already compressed (e.g. a btrfs or ZFS
volume with compression enabled) or when CPU is tightly
constrained.

### Changing compression_type on an existing spool

The compression algorithm is recorded per-SST at write time;
RocksDB happily mixes SSTs written with different algorithms in
the same database.  Changing this option therefore does **not**
require any migration step:

* SSTs already on disk continue to be read using whatever
  algorithm they were originally written with.
* New SSTs (memtable flushes and compaction output) are written
  using the newly-configured algorithm.
* Over time, normal background compaction rewrites old SSTs and
  the database gradually migrates to the new algorithm.  You can
  force the migration to complete immediately by running
  [kcli spool-compact](../../kcli/spool-compact.md), at the cost
  of pausing background work for the duration of the compaction.

The one caveat is that the build of `kumod` you switch to must
still link the algorithm used by your existing SSTs.  Because the
official builds only include `"None"` and `"Snappy"`, the only
safe switches without a custom build are between those two
values.

If disk-space budgeting is a concern, note that switching from
Snappy to `"None"` increases the on-disk footprint as old SSTs
are rewritten by compaction.  Plan capacity before flipping the
switch in that direction.

## compaction_readahead_size

{{since('2023.03.31-36aa20de')}}

If non-zero, RocksDB performs larger reads when doing compaction.
If you're running RocksDB on spinning disks, you should set this to
at least 2MB so that RocksDB's compaction is doing sequential
instead of random reads.

## level_compaction_dynamic_level_bytes

{{since('2023.03.31-36aa20de')}}

When `true`, RocksDB dynamically picks target sizes for each level
to keep the overall LSM tree balanced even under heavily skewed
write patterns.  Recommended for level-style compaction with
variable load.  Defaults to `false`.

## max_open_files

{{since('2023.03.31-36aa20de')}}

Upper bound on the number of SST files RocksDB may keep open
simultaneously.  When unset, RocksDB defaults to keeping every SST
file open, which gives the best read latency but can exhaust file
descriptors on very large spools.  Set this to bound descriptor
use; closed files are reopened on demand at a small per-read cost.

## write_buffer_size

{{since('dev')}}

Size in bytes of the rocksdb memtable that buffers writes before
being flushed to disk as a new SST file.

Smaller values produce smaller, more frequent SST files and
trigger compactions sooner -- useful in test setups that need to
force the storage through its full write/compact lifecycle
quickly.  Larger values amortize compaction overhead but increase
memory use and recovery time after restart.  Leave unset to use
the rocksdb default.

## level0_stop_writes_trigger

{{since('dev')}}

Number of level-0 SST files at which rocksdb will stop accepting
writes.  Lower values transition the database into the
write-stopped state more quickly when background compaction cannot
keep up, which is useful for tests that need to deterministically
observe that condition.  Leave unset to use the rocksdb default.

## log_level

{{since('2023.03.31-36aa20de')}}

Controls the verbosity of the RocksDB `LOG` file that lives
alongside the SST files in the spool directory.  Possible values:
`"Debug"`, `"Info"` (the default), `"Warn"`, `"Error"`, `"Fatal"`,
`"Header"`.  `Info` is verbose enough to surface background
compaction errors and is generally what you want; raise to `Warn`
or `Error` only if log volume is a concern.

## memtable_huge_page_size

{{since('2023.03.31-36aa20de')}}

When set, RocksDB attempts to allocate memtable memory backed by
transparent huge pages of the requested size, reducing TLB pressure
on systems with large memtables.  See the rocksdb [`set_memtable_huge_page_size`](https://docs.rs/rocksdb/latest/rocksdb/struct.Options.html#method.set_memtable_huge_page_size)
documentation for caveats.  Defaults to unset (use rocksdb's
default of regular pages).

## log_file_time_to_roll

{{since('2023.11.28-b5252a41')}}

How long a RocksDB `LOG` file may be appended to before it is
rolled to a new file.  Specified as a duration string (e.g.
`"24 hours"`).  Defaults to 24 hours.

## obsolete_files_period

{{since('2023.11.28-b5252a41')}}

How often RocksDB scans for and deletes obsolete SST and log
files.  Specified as a duration string (e.g. `"6 hours"`).
Defaults to 6 hours.  Smaller values reclaim disk space sooner at
the cost of additional periodic scanning work.

## limit_concurrent_stores

{{since('2025.03.19-1d3f1f67')}}

When saving to RocksDB, KumoMTA first attempts the write with
RocksDB's `no_slowdown` flag set.  If memtable backpressure causes
RocksDB to reject the immediate attempt, KumoMTA polls with a
short backoff until the write is accepted or the deadline expires
(see [store_deadline](#store_deadline)).

This option caps the number of concurrent store calls that are
allowed to hold a backpressure-retry slot at the same time.  In a
healthy spool no slot is held -- the first attempt succeeds
immediately.  Under backpressure, the cap prevents an unbounded
number of in-flight retrying tasks from amplifying contention on
the memtable lock.

You should experiment to find which setting works best for your
workload, but the recommended starting point is shown below:

```lua
kumo.on('init', function()
  local params = {
    -- Use double the number of cores
    limit_concurrent_stores = kumo.available_parallelism() * 2,
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## limit_concurrent_loads

{{since('2025.03.19-1d3f1f67')}}

RocksDB does not have a non-blocking read API: `load()` always
runs the underlying `get()` call in a blocking task on the tokio
blocking pool, since the I/O wait must happen on some thread.

By default there is no spool-specific upper bound on the number
of outstanding loads, and the blocking pool can grow to up to 512
threads.

Setting an explicit cap reduces CPU contention and context
switches when many loads are in flight, at the cost of bounding
throughput when the cache is cold and most loads have to wait on
disk I/O.  When set, this option defines a semaphore that limits
how many `load()` calls may be running -- counting both the
waiting and the executing tasks -- at the same time.

You should experiment to find which setting works best for your workload,
but the recommended starting point is shown below:

```lua
kumo.on('init', function()
  local params = {
    -- Use double the number of cores
    limit_concurrent_loads = kumo.available_parallelism() * 2,
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## limit_concurrent_removes

{{since('2025.03.19-1d3f1f67')}}

When deleting from RocksDB, KumoMTA first attempts the delete with
RocksDB's `no_slowdown` flag set.  If memtable backpressure causes
RocksDB to reject the immediate attempt, KumoMTA polls with a
short backoff until the delete is accepted or the deadline expires
(see [store_deadline](#store_deadline)).

This option caps the number of concurrent remove calls that are
allowed to hold a backpressure-retry slot at the same time.  In a
healthy spool no slot is held -- the first attempt succeeds
immediately.  Under backpressure, the cap prevents an unbounded
number of in-flight retrying tasks from amplifying contention on
the memtable lock.

You should experiment to find which setting works best for your
workload, but the recommended starting point is shown below.

```lua
kumo.on('init', function()
  local params = {
    -- Use double the number of cores
    limit_concurrent_removes = kumo.available_parallelism() * 2,
  }
  kumo.define_spool {
    name = 'data',
    path = '/var/spool/kumo/data',
    kind = 'RocksDB',
    rocks_params = params,
  }
  kumo.define_spool {
    name = 'meta',
    path = '/var/spool/kumo/meta',
    kind = 'RocksDB',
    rocks_params = params,
  }
end)
```

## store_deadline

{{since('dev')}}

Upper bound on the wait that `store()` and `remove()` will tolerate
when rocksdb is applying backpressure.  Specified as a duration
string (e.g. `"30s"`).  Defaults to 30 seconds.

Callers may provide a shorter deadline (typically derived from an
SMTP client's idle timeout); the effective deadline is the minimum
of the two.  Going longer than the caller-provided value risks the
client timing out and retrying, which would produce duplicate
deliveries -- this option therefore only narrows the effective
deadline, it never extends it.

When the caller-provided deadline is the one that fires, the SMTP
server surfaces a `4.4.5 data_processing_timeout exceeded` response
to the peer.  When this spool-side deadline is the one that fires,
the SMTP server surfaces `4.4.5 spool write timed out` instead, so
operators can tell the two cases apart.

## error_latch_duration

{{since('dev')}}

How long the composite "this database is wedged" signal must hold
continuously before the load-shedding gate latches.  Specified as
a duration string.  Defaults to 15 seconds.

The signal goes high whenever the rocksdb `background-errors`
counter has grown above the value observed at process start, or
any foreground spool operation has returned a rocksdb error since
process start; see
[rocks_spool_load_shed_active](../../metrics/kumod/rocks_spool_load_shed_active.md)
for the full description.  Brief blips that recover within this
window do not latch the gate, which filters out transient
auto-resumed errors.

Note that fatal foreground errors (`Corruption` or `IOError`)
latch the gate immediately and do not consult this debounce
window.

## error_unlatch_duration

{{since('dev')}}

How long the healthy state must hold continuously before the
load-shedding gate auto-unlatches.  Specified as a duration string.
Defaults to 5 minutes.  Only consulted when
[allow_error_unlatch](#allow_error_unlatch) is `true`.

A relatively long value gives operators time to inspect the
database after a brief failure window before the daemon starts
accepting writes again on its own.

## allow_error_unlatch

{{since('dev')}}

When `true` (the default), the load-shedding gate clears itself
after [error_unlatch_duration](#error_unlatch_duration) of observed
recovery.

Set to `false` to require an operator restart to clear the gate,
which is appropriate when you want a human to confirm the
underlying cause is resolved before accepting traffic again.
