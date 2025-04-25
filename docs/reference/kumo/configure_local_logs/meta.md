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

See also:

  * [msg:import_x_headers](../../message/import_x_headers.md)
  * [msg:set_meta](../../message/set_meta.md)
