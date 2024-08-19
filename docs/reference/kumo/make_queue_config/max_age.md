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


