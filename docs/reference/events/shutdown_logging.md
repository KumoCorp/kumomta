# shutdown_logging

```lua
kumo.on('shutdown_logging', function() end)
```

{{since('2025.10.06-5ec871ab')}}

Called by `kumod` as part of shutdown, just prior to shutting down all loggers.

The intended use case is for you to be able to `close` any mpsc queues that you
might have defined in your policy, which in turn allows for a more graceful
shutdown:

```lua
local kumo = require 'kumo'

kumo.on('init', function()
  kumo.spawn_task {
    event_name = 'logger',
    args = {},
  }
end)

kumo.on('logger', function(args)
  local q = kumo.mpsc.define 'queue'
  while true do
    local batch = q:recv_many(100)
    if not batch then
      print 'logger loop done; shutting down'
      return
    end
  end
end)

kumo.on('shutdown_logging', function()
  local q = kumo.mpsc.define 'queue'
  q:close()
end)
```
