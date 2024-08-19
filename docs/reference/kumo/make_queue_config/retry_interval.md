# retry_interval

Messages are retried using an exponential backoff.  *retry_interval* sets the
base interval; if a message cannot be immediately delivered and encounters a
transient failure, then a (jittered) delay of *retry_interval* seconds will be
applied before trying again. If it transiently fails a second time,
*retry_interval* will be doubled and so on, doubling on each attempt.

The default is `"20 minutes"`.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  return kumo.make_queue_config {
    retry_interval = '20 minutes',
  }
end)
```


