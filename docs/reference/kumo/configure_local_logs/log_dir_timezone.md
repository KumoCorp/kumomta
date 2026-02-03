tags:
 - logging
---

# log_dir_timezone

Selects the timezone used when expanding `strftime` directives inside
[`log_dir`](log_dir.md) or [`per_record.log_dir`](per_record.md).

```lua
kumo.configure_local_logs {
  log_dir = '/var/log/kumo-logs/%Y/%m/%d',
  log_dir_timezone = 'UTC',
}
```

* `UTC` (default) renders directory patterns with Coordinated Universal Time,
  matching the historic behavior of KumoMTA log directories and ensuring a
  consistent layout across hosts in different time zones.
* `Local` uses the server's local clock to compute the directory path, which is
  helpful when you want the hierarchy to follow the output of the `date`
  command on that machine.

The timezone setting applies to both the initial directory creation at startup
and every subsequent segment write; when a dynamic path crosses a boundary (for
example, a new day in UTC), the logger closes old handles and opens the new
path automatically.
