# max_age

Limits how long a message can remain in the queue.
The default value is `"7 days"`.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    -- Age out messages after being in the queue for 20 minutes
    max_age = '20 minutes',
  }
end)
```

!!! note
    If you are using [message:set_scheduling()](../../message/set_scheduling.md)
    to configure a custom `expires` timestamp on a per-message basis, then
    `max_age` will be ignored for those messages and only your `expires` timestamp
    will be considered for expiration.
