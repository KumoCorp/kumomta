---
tags:
 - logging
---

# max_segment_duration

Specify the maximum time period for a file segment.  The default is unlimited.

If you set this to `"1min"`, you indicate that any given file should cover a
time period of 1 minute in duration; when that time period elapses, the current
file segment, if any, will be flushed and closed and any subsequent events will
cause a new file segment to be created.

```lua
kumo.configure_local_logs {
  -- ..
  max_segment_duration = '5 minutes',
}
```


