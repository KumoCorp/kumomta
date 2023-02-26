# `kumo.on('get_queue_config', function(queue_name))`

```admonish
This event handler is in flux and may change significantly
```

Not the final form of this API, but this is currently how
we retrieve configuration used for managing a queue.

```lua
kumo.on('get_queue_config', function(queue_name)
  return kumo.make_queue_config {
    max_retry_interval = 20 * 60,
  }
end)
```

See also [kumo.make_queue_config](../kumo/make_queue_config.md).
