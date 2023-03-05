# `kumo.configure_local_logs {PARAMS}`

Enables local logging of reception and delivery events to the specified
`log_dir` directory.

Logs are written as zstd-compressed log file segments under the specified
directory.  Each line of the file is a JSON object holding information about
a reception or delivery related event.

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## back_pressure

Maximum number of outstanding items to be logged before
the submission will block; helps to avoid runaway issues
spiralling out of control.

```lua
kumo.configure_local_logs {
  -- ..
  back_pressure = 128000,
}
```

## compression_level

Specifies the level of *zstd* compression that should be used.  Compression
cannot be disabled.

Specifying `0` uses the zstd default compression level, which is `3` at the
time of writing.

Possible values are `1` (cheapest, lightest) through to `21`.

```lua
kumo.configure_local_logs {
  -- ..
  compression_level = 3,
}
```

## log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.configure_local_logs {
  -- ..
  log_dir = '/var/log/kumo-logs',
}
```

## max_file_size

Specify how many uncompressed bytes to allow per file segment. When this number
is exceeded, the current segment is finished and a new segment is created.

Segments are created using the current time in the form `YYYYMMDD-HHMMSS` so that
it is easy to sort the segments in chronological order.

The default value is ~1GB of uncompressed data, which compresses down to around
50MB of data per segment with the default compression settings.

```lua
kumo.configure_local_logs {
  -- ..
  max_file_size = 1000000000,
}
```

## max_segment_duration

Specify the maximum time period for a file segment.  The default is unlimited.

If you set this to `"1min"`, you indicate that any given file should cover a
time period of 1 minute in duration; when that time period elapses, the current
file segment, if any, will be flushed and closed and any subsequent events will
cause a new file segment to be created.

```lua
kumo.configure_local_logs {
  -- ..
  max_segment_duration = "5 minutes",
}
```
