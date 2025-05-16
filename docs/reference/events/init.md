---
title: init
---

# `kumo.on('init', FUNCTION)`

The `init` event is triggered once when the `kumod` process initializes.
The intent is that you use this event to define the spool storage and
listeners for your environment.

The event handler is not passed any parameters, and does not expect
any particular return value.

```lua
-- Called on startup to initialize the system
kumo.on('init', function()
  -- Define a listener.
  -- Can be used multiple times with different parameters to
  -- define multiple listeners!
  kumo.start_esmtp_listener {
    listen = '0.0.0.0:2025',
  }

  kumo.configure_local_logs {
    log_dir = '/var/tmp/kumo-logs',
  }

  kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }

  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
  }

  kumo.define_spool {
    name = 'meta',
    path = '/var/tmp/kumo-spool/meta',
  }
end)
```
