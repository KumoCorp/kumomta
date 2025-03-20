# rocks_params

Specifies additional tuning parameters for RocksDB when `kind = "RocksDB"`.

The following parameters are possible:

## increase_parallelism

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

## limit_concurrent_loads

{{since('2025.03.19-1d3f1f67')}}

When loading from RocksDB, KumoMTA will first try a non-blocking load, but if
RocksDB is too busy for that, a blocking load will occur in a background thread
pool.

By default there is no spool-specific upper bound to the number of outstanding
blocking submission tasks, and the thread pool in which they run can grow to
up to 512 threads.

You will generally benefit from improved latency and low CPU contention and
context switches if you set an upper bound on the number of outstanding
store tasks that are permitted.

That is where this option comes in; when specified, it defines a semaphore
that will limit the number of tasks that are spawned into the blocking
thread pool.

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

When deleting from RocksDB, KumoMTA will first try a non-blocking remove, but if
RocksDB is too busy for that, a blocking remove will occur in a background thread
pool.

By default there is no spool-specific upper bound to the number of outstanding
blocking submission tasks, and the thread pool in which they run can grow to
up to 512 threads.

You will generally benefit from improved latency and low CPU contention and
context switches if you set an upper bound on the number of outstanding
store tasks that are permitted.

That is where this option comes in; when specified, it defines a semaphore
that will limit the number of tasks that are spawned into the blocking
thread pool.

You should experiment to find which setting works best for your workload,
but the recommended starting point is shown below.

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

## limit_concurrent_stores

{{since('2025.03.19-1d3f1f67')}}

When saving to RocksDB, KumoMTA will first try a non-blocking submission, but if
RocksDB is too busy for that, a blocking submission will occur in a background
thread pool.

By default there is no spool-specific upper bound to the number of outstanding
blocking submission tasks, and the thread pool in which they run can grow to
up to 512 threads.

You will generally benefit from improved latency and low CPU contention and
context switches if you set an upper bound on the number of outstanding
store tasks that are permitted.

That is where this option comes in; when specified, it defines a semaphore
that will limit the number of tasks that are spawned into the blocking
thread pool.

You should experiment to find which setting works best for your workload,
but the recommended starting point is shown below:

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


