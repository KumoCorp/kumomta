---
tags:
 - logging
---

# max_file_size

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


