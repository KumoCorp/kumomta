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
