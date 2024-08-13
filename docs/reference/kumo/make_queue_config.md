# `kumo.make_queue_config { PARAMS }`

Constructs a configuration object that specifies how a *queue* will behave.

This function should be called from the
[get_queue_config](../events/get_queue_config.md) event handler to provide the
configuration for the requested queue.

The following keys are possible:

## egress_pool

The name of the egress pool which should be used as the source of
this traffic.

If you do not specify an egress pool, a default pool named `unspecified`
will be used. That pool contains a single source named `unspecified` that
has no specific source settings: it will just make a connection using
whichever IP the kernel chooses.

See [kumo.make_egress_pool()](make_egress_pool.md).

## max_age

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

## max_message_rate

{{since('2024.06.10-84e84b89')}}

Optional string.

Specifies the maximum permitted rate at which messages can move from this
scheduled queue and into the ready queue for the appropriate egress source.

The value is of the form `quantity/period`
where quantity is a number and period can be a measure of time.

Examples of throttles:

```
"10/s" -- 10 per second
"10/sec" -- 10 per second
"10/second" -- 10 per second

"50/m" -- 50 per minute
"50/min" -- 50 per minute
"50/minute" -- 50 per minute

"1,000/hr" -- 1000 per hour
"1_000/h" -- 1000 per hour
"1000/hour" -- 1000 per hour

"10_000/d" -- 10,000 per day
"10,000/day" -- 10,000 per day
```

Throttles are implemented using a Generic Cell Rate Algorithm.

If the throttle is exceeded the message will be re-inserted into the scheduled
queue with a delay based on the acceptance rate of the throttle.

This option is distinct from [the egress path
max_message_rate](make_egress_path.md#max_message_rate) option in that this one
applies to a specific scheduled queue, whilst the egress path option applies to
the ready queue for a specific egress path, through which multiple scheduled
queues send out to the internet.

If you have configured `max_message_rate` both here and in an egress path,
the effective maximum message rate will be the lesser of the two values; both
constraints are applied independently from each other at different stages
of processing.

## max_retry_interval

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

## protocol

Configure the delivery protocol. The default is to use SMTP to the
domain associated with the queue, but you can also configure delivering
to a local [maildir](http://www.courier-mta.org/maildir.html), or using
custom lua code to process a message

### Example of smart-hosting with the SMTP protocol

{{since('2023.08.22-4d895015')}}

Rather than relying on MX resolution, you can provide an explicit list
of MX host names or IP addresses to which the queue should deliver.
The addresses will be tried in the order specified.

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'smarthost.example.com' then
    -- Relay via some other internal infrastructure.
    -- Enclose IP (or IPv6) addresses in `[]`.
    -- Otherwise the name will be resolved for A and AAAA records
    return kumo.make_queue_config {
      protocol = {
        smtp = {
          mx_list = {
            'smart.host.local',
            { name = 'mx.example.com', addr = '10.0.0.1' },
          },
        },
      },
    }
  end
  -- Otherwise, just use the defaults
  return kumo.make_queue_config {}
end)
```

### Example of using the Maildir protocol

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'maildir.example.com' then
    -- Store this domain into a maildir, rather than attempting
    -- to deliver via SMTP
    return kumo.make_queue_config {
      protocol = {
        maildir_path = '/var/tmp/kumo-maildir',
      },
    }
  end
  -- Otherwise, just use the defaults
  return kumo.make_queue_config {}
end)
```

!!! note
    Maildir support is present primarily for functional validation
    rather than being present as a first class delivery mechanism.

Failures to write to the maildir will cause the message to be delayed and
retried approximately 1 minute later.  The normal message retry schedule does
not apply.

### Using Lua as a delivery protocol

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'webhook' then
    -- Use the `make.webhook` event to handle delivery
    -- of webhook log records
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          -- this will cause an event called `make.webhook` to trigger.
          -- You can pick any name for this event, so long as it doesn't
          -- collide with a pre-defined event, and so long as you bind
          -- to it with a kumo.on call
          constructor = 'make.webhook',
        },
      },
    }
  end
  return kumo.make_queue_config {}
end)

-- This event will be called each time we need to make a connection.
-- It needs to return a lua object with a `send` method
kumo.on('make.webhook', function(domain, tenant, campaign)
  -- Create the connection object
  local connection = {}

  -- define a send method on the connection object.
  -- The return value is the disposition string for a successful
  -- delivery; that string will get logged in the resulting log record.
  -- If the delivery failed, you can use `kumo.reject` to raise the
  -- error with an appropriate 400 or 500 code.
  -- 400 codes will be retried later. 500 codes will log a permanent
  -- failure and no further delivery attempts will be made for the message.
  function connection:send(message)
    print(message:get_data())
    if failed then
      kumo.reject(400, 'failed for some reason')
    end
    return 'OK'
  end

  return connection
end)
```

See [should_enqueue_log_record](../events/should_enqueue_log_record.md) for
a more complete example.


## retry_interval

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

## reap_interval

{{since('dev')}}

Optional duration string. The default is `"10m"`.  It controls how long the
queue should remain empty and idle before we reap it from the queue management
layer and free its associated resources and metrics.

## refresh_interval

{{since('dev')}}

Optional duration string. The default is `"1m"`.  It controls how long the
queue should wait before refreshing the configuration for that queue by
triggering the [get_queue_config](../events/get_queue_config.md) event.

## strategy

{{since('dev')}}

Optional string to specify the scheduled queue strategy.  There are two possible
values:

* `"TimerWheel"` - the default. The timer wheel has `O(1)` insertion and `O(1)`
  pop costs, making it good for very large scheduled queues, but that comes in
  exchange for a flat periodic tick overhead.  As the number of scheduled queues
  increases and/or the `retry_interval` decreases, so does the aggregate overhead
  of maintaining timerwheel queues.
* `"SkipList"` - A skiplist has `O(log n)` insertion and `O(1)` pop costs,
  making it a good general purpose queue, but becomes more expensive to insert
  into as the size of the queue increases.  That higher insertion cost is in
  exchange for the overall maintenance being cheaper, as the queue can go to
  sleep until the time of the next due item.  The ongoing and aggregate
  maintenance is therefore cheaper than a `TimerWheel` but the worst-case
  scenario where the destination is not accepting mail and causing the
  scheduled queue to grow is logarithmically more expensive as the queue
  grows.

Which should you use? Whichever works best for your environment! Make sure that
you test normal healthy operation with a lot of queues as well as the worst
case scenario where those queues are full and egress is blocked.

If you have very short `retry_interval` set for the majority of your queues you
may wish to adopt `SkipList` for its lower idle CPU cost, or alternatively use
`TimerWheel` and find a `timerwheel_tick_interval` that works for your typical
number of queues.

!!! note
    Changing the strategy for a given queue requires that the queue either be
    aged out, or for kumod to be restarted, before it will take effect.  This
    restriction may be removed in a future release.

## timerwheel_tick_interval

{{since('dev')}}

When using the default `strategy = "TimerWheel"`, the timer wheel needs to
be ticked regularly in order to promote messages into the ready queue. The default
tick interval is computed as `retry_interval / 20` and clamped to be within the
range `>= 1s && <= 1m`.

If you have a short `retry_interval` and a lot of scheduled queues you may find
that your system is spending more time ticking over than is desirable, so you can
explicitly select the tick interval via this option.

The value is an optional string duration like `1m`.

If you have to set this, our recommendation is generally for this to be as long
as possible.

!!! note
    The maintainer will also tick over whenever the
    [refresh_interval](#refresh_interval) elapses, so there isn't a tangible
    benefit to setting `timerwheel_tick_interval` larger than `refresh_interval`.
