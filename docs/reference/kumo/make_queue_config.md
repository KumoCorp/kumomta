# `kumo.make_queue_config { PARAMS }`

Constructs a configuration object that specifies how a *queue* will behave.

This function should be called from the
[get_queue_config](../events/get_queue_config.md) event handler to provide the
configuration for the requested queue.

The following keys are possible:

## max_age

Limits how long a message can remain in the queue.
The value is expressed in seconds.  The default value is 7 days.

```lua
kumo.on('get_queue_config', function(queue_name)
  return kumo.make_queue_config {
    -- Age out messages after being in the queue for 20 minutes
    max_age = 20 * 60,
  }
end)
```

## max_retry_interval

Messages are retried using an exponential backoff as described under
*retry_interval* below. *max_retry_interval* sets an upper bound on the amount
of time between delivery attempts.

The default is that there is no upper limit.

The value is expressed in seconds.

```lua
kumo.on('get_queue_config', function(queue_name)
  return kumo.make_queue_config {
    -- Retry at most every hour
    max_retry_interval = 60 * 60,
  }
end)
```

## retry_interval

Messages are retried using an exponential backoff.  *retry_interval* sets the
base interval; if a message cannot be immediately delivered and encounters a
transient failure, then a (jittered) delay of *retry_interval* seconds will be
applied before trying again. If it transiently fails a second time,
*retry_interval* will be doubled and so on, doubling on each attempt.

The default is 20 minutes.

The value is expressed in seconds.

```lua
kumo.on('get_queue_config', function(queue_name)
  return kumo.make_queue_config {
    -- 20 minutes
    retry_interval = 20 * 60,
  }
end)
```
