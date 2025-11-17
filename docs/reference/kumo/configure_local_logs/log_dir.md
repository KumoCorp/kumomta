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

The directory can also be expressed as a [`strftime`-style
pattern](https://docs.rs/chrono/latest/chrono/format/strftime/), allowing
segments to be routed into dated hierarchies using `%` directives. Literal
percent signs can be produced using `%%`.

```lua
kumo.configure_local_logs {
  log_dir = '/var/log/kumo-logs/%Y/%m/%d',
}
```

With the example above, delivery log segments created at midnight on 15 Nov
2025 would be written below `/var/log/kumo-logs/2025/11/15` with a final
result like

```
.
└── 2025
        └── 11
                ├── 14
                │       ├── 20251114-162000.001322016
                │       └── 20251114-162010.013448752
                └── 15
                        ├── 20251115-151900.001392018
                        ├── 20251115-151910.003492751
                        ├── 20251115-151920.005733515
                        ├── 20251115-151930.009024998
                        └── 20251115-151940.010643308
```

Patterns are evaluated in UTC by default, so directives such as `%Y`, `%m`, `%d`,
or `%H` reflect Coordinated Universal Time. To follow the host's wall-clock
time instead, set [`log_dir_timezone`](log_dir_timezone.md) to `"Local"`.

Any directive supported by `chrono`'s `strftime` formatter may be used, so `%s`
captures the Unix timestamp, `%H` the hour, and so on.

The same syntax is valid inside [`per_record.log_dir`](per_record.md), allowing
individual record types to be routed to their own time-based directories.
