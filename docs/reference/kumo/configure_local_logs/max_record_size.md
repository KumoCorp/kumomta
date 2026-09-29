---
tags:
 - logging
---

# max_record_size

{{since('2026.09.29-b90d8bc1')}}

Specify the largest a log record and its newline separator may be, in bytes. A
record that would not fit is dropped rather than written, which keeps segments
readable by tools that bound how much of a record they will buffer. Because the
record plus its separator must fit, a record must be smaller than this value.

Dropped records are counted by the `log_record_dropped_too_large` metric,
labelled by `log_dir`. An error-level log line is emitted when records are
dropped, rate limited to at most one line per minute. It reports the count and
the kind, id, and size of the most recent dropped record, but never the record
body.

Defaults to `134217728` (128 MiB), the same limit that
[`kumo.jsonl.new_tailer`](../../kumo.jsonl/new_tailer.md) accepts by default, so
a record too large to be read back is dropped here rather than producing a
segment that cannot be tailed. There is always a limit. To permit larger
records, raise it (and the tailer's `max_line_size` to match).

```lua
kumo.configure_local_logs {
  -- ..
  max_record_size = 134217728,
}
```
