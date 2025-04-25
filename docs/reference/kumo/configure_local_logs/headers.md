---
tags:
 - logging
---

# headers

Specify a list of message headers to include in the logs. The default is
empty.

!!! warning
    While logging headers directly is convenient and easy to express in the
    logging configuration, it comes with additional runtime CPU and IO
    overhead: every discrete event that is logged will result in the message
    contents being loaded from spool (if they are not already loaded), the
    message headers being parsed, and the selected headers decoded to be logged.

    If your system has CPU and IO to spare, this is a non-issue, but
    if you are pushing your system to its limits, and especially if
    you have a large scheduled queue with lots of throttled or otherwise
    delayed messages, these overheads can dominate the system and harm
    overall throughput.

    We recommend instead using
    [msg:import_x_headers()](../../message/import_x_headers.md) during message
    reception to cache a copy of the headers that you desire to log into your
    message metadata, then listing those metadata fields in your logger
    [meta](meta.md) list *instead* of using `headers`.  This will dramatically
    reduce the IO and CPU overheads around logging.

```lua
kumo.configure_local_logs {
  -- You can log headers like this, but it is not recommended!
  -- You generally should prefer to log `meta` instead.
  headers = { 'Subject' },
}
```

Please consider using [meta](meta.md) rather than `headers`!

{{since('2023.12.28-63cde9c7', indent=True)}}
    Header names can now use simple wildcard suffixes; if the last character
    of the header name is `*` then it will match any string with that prefix.
    For example `"X-*"` will match any header names that start with `"X-"`.

