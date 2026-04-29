---
tags:
 - utility
 - logging
 - jsonl
---

# `kumo.jsonl.new_writer`

```lua
local writer = kumo.jsonl.new_writer {
  log_dir = '/path/to/logs',
  -- optional fields:
  max_file_size = 134217728,
  compression_level = 3,
  max_segment_duration = '1h',
  suffix = '.zst',
  tz = 'America/New_York',
}
```

{{since('2026.04.09-ea3b2a9b')}}

Creates a new `LogWriter` that writes zstd-compressed JSONL segment files into
`log_dir`.  Each record occupies one line in the decompressed output.

When the writer is closed (either explicitly via `:close()` or implicitly via
the `<close>` attribute), the current segment is finalized and marked as
read-only so that a tailer can detect that it is complete.

## Configuration Parameters

### log_dir

*String.* Required. The directory in which segment files will be created.
The directory is created automatically if it does not already exist.

### max_file_size

*Integer.* Optional. Maximum number of **uncompressed** bytes to write before
rolling to a new segment file. Defaults to `134217728` (128 MiB).

### compression_level

*Integer.* Optional. The zstd compression level to use. `0` selects the zstd
library default. Valid explicit levels are `1`–`21`. Defaults to `3`.

### max_segment_duration

*Duration string* (e.g., `"1h"`, `"30m"`, `"90s"`). Optional. Maximum time a
segment file remains open before it is closed and a new one is started, even
if `max_file_size` has not been reached. If omitted, segments are only rolled
on size.

### suffix

*String.* Optional. A suffix appended to each segment file name. For example,
`".zst"` will produce names like `20240101-120000.000000000.zst`.

### tz

*Timezone name string* (e.g., `"America/New_York"`, `"Europe/London"`).
Optional. The timezone used when computing the timestamp portion of the
segment file name. Defaults to UTC.

## Methods

### writer:write_line

```lua
writer:write_line(line)
```

Writes `line` (a string) as a single JSONL record.  A trailing newline is
appended automatically if `line` does not already end with one.

Raises an error if the writer has been closed.

### writer:write_record

```lua
writer:write_record(value)
```

Serializes the lua `value` to JSON and writes it as a JSONL record.
Equivalent to calling `writer:write_line(kumo.json_encode(value))`.

Raises an error if the writer has been closed.

### writer:close

```lua
writer:close()
```

Flushes and finalizes the current segment file, marking it as read-only (done)
so that a tailer knows it is complete.  Subsequent calls to `:write_line` or
`:write_record` will open a new segment.

## Example

```lua
local kumo = require 'kumo'

local writer = kumo.jsonl.new_writer {
  log_dir = '/var/log/kumomta/json',
  max_file_size = 268435456, -- 256 MiB
  max_segment_duration = '1h',
  compression_level = 3,
  suffix = '.zst',
}

writer:write_record { event = 'Delivery', id = 'abc123' }
writer:write_record { event = 'Bounce', id = 'def456' }
writer:close()
```
