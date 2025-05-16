# kumo.configure_redis_throttles

```lua
kumo.configure_redis_throttles { PARAMS }
```

Configure the throttle layer to use a [Redis](https://redis.io/) data store to
manage throttling across multiple MTA nodes.

When running version 2024.09.02-c5476b89 or earlier, the redis server must have
[redis-cell](https://github.com/brandur/redis-cell) installed for throttles to
be shared. Later versions will automatically detect whether `redis-cell` is
available and fall back to an alternative throttling implementation that
doesn't have any other additional dependency requirements for the redis server.

*PARAMS* behaves exactly as described in [redis.open](../redis/open.md).

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  -- Use shared throttles and connection limits rather than in-process throttles
  kumo.configure_redis_throttles { node = 'redis://my-redis-host/' }
end)
```

{{since('2023.08.22-4d895015', indent=True)}}
    Enabling redis throttles now also enables redis-based shared
    connection limits.
