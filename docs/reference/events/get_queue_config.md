# `kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain))`

!!! note
    This event handler is in flux and may change significantly

Not the final form of this API, but this is currently how
we retrieve configuration used for managing a queue.

The parameters correspond to the `domain`, `tenant`, `campaign` and `routing_domain`
fields from the *scheduled queue* name, as discussed in [Queues](../queues.md).

```lua
kumo.on('get_queue_config', function(domain_name, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    max_retry_interval = '20 minutes',
  }
end)
```

See also [kumo.make_queue_config](../kumo/make_queue_config.md).
