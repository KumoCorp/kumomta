# `kumo.configure_redis_throttles { PARAMS }`

Configure the throttle layer to use a [Redis](https://redis.io/) data store to
manage throttling across multiple MTA nodes.

The redis server must have [redis-cell](https://github.com/brandur/redis-cell)
installed for throttles to work in this way.

*PARAMS* behaves exactly as described in [redis.open](../redis/open.md).

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  -- Use shared throttles rather than in-process throttles
  kumo.configure_redis_throttles { node = 'redis://my-redis-host/' }
end)
```
