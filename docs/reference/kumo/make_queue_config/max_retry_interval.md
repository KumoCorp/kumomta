# max_retry_interval

Messages are retried using an exponential backoff as described under
*retry_interval* below. *max_retry_interval* sets an upper bound on the amount
of time between delivery attempts.

The default is that there is no upper limit.

The value is expressed in seconds.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    -- Retry at most every hour
    max_retry_interval = '1 hour',
  }
end)
```


