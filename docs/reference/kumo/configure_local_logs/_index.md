---
tags:
 - logging
---

# kumo.configure_local_logs

```lua
kumo.configure_local_logs { PARAMS }
```

Enables local logging of reception and delivery events to the specified
`log_dir` directory.

Logs are written as zstd-compressed log file segments under the specified
directory.  Each line of the file is a JSON object holding information about
a reception or delivery related event.  The format of the Log Record object
can be found [here](../../log_record.md).

This function should be called only from inside your [init](../../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## Local Log File Parameters { data-search-exclude }
