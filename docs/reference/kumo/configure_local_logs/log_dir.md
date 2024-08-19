# log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.configure_local_logs {
  -- ..
  log_dir = '/var/log/kumo-logs',
}
```


