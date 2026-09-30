---
tags:
 - logging
---

# log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.configure_local_logs {
  -- ..
  log_dir = '/var/log/kumo-logs',
}
```

!!! note
    A `log_dir` must hold one log stream. Use a separate directory for
    each stream, whether the streams come from `per_record`
    [`suffix`](per_record.md) values or from more than one
    [`kumo.jsonl.new_writer`](../../kumo.jsonl/new_writer.md).


