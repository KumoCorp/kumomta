---
tags:
 - logging
 - meta
---

# meta

Specify a list of message meta fields to include in the logs. The default is
empty.

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    -- ..
    meta = { 'my-meta-1', 'subject' },
  }
end)

kumo.on('smtp_server_message_received', function(msg, conn_meta)
  -- Arrange to log the subject header in the most
  -- efficient way, by capturing it into the message
  -- metadata when we receive the message.
  -- The `msg:import_x_headers` method will capture non-x-header
  -- names when header names are explicitly passed.
  msg:import_x_headers { 'subject' }

  -- set an arbitrary meta item; it will be logged because
  -- `my-meta-1` is listed in the logging configuration
  -- above.
  msg:set_meta('my-meta-1', 'some value')
end)
```

{{since('dev', indent=True)}}
    Meta names can now use simple wildcard suffixes; if the last character
    of the meta name is `*` then it will match any string with that prefix.
    For example `"xfer_*"` will match any meta names that start with `"xfer_"`.

!!! note
    meta names are case sensitive, so you must specify the exact matching
    case, even if you are using wildcards to match names.

See also:

  * [msg:import_x_headers](../../message/import_x_headers.md)
  * [msg:set_meta](../../message/set_meta.md)
