# Routing Messages via Kafka

{{since('2023.12.28-63cde9c7')}}

In addition to local logging and Webhooks, KumoMTA can relay log events (or other queued messages) via [Apache Kafka](https://kafka.apache.org/).

KumoMTA supports publishing via Kafka, using Lua.

The process to queue log events and make them available for sending via `custom_lua` as a protocol is covered in the [Publishing Log Events Via Webhooks](../operation/webhooks.md) section of the Operations chapter of the User Guide.

## Configuring A Queue Handler for Kafka

When a message is ready to be queued, the `get_queue_config` event is fired, at which point we can specify the protocol of the queue, in this case, `custom_lua`. In the example below, we check whether the message is queued to the `kafka` queue and acts accordingly:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'kafka' then
    -- Use the `make.webhook` event to handle delivery
    -- of webhook log records
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          -- this will cause an event called `make.webhook` to trigger.
          -- You can pick any name for this event, so long as it doesn't
          -- collide with a pre-defined event, and so long as you bind
          -- to it with a kumo.on call
          constructor = 'make.kafka',
        },
      },
    }
  end
  return kumo.make_queue_config {}
end)
```

## Sending Messages via Kafka

With the custom_lua protocol defined and a custom event trigger declared, the next step is to catch the `make.kafka` event with code that sends the message contents over HTTP.

The following example sends the content of the log message via Kafka:

```lua
-- This is a user-defined event that matches up to the custom_lua
-- constructor used in `get_queue_config` below.
-- It returns a lua connection object that can be used to "send"
-- messages to their destination.
kumo.on('make.kafka', function(domain, tenant, campaign)
  local producer = kumo.kafka.build_producer {
    ['bootstrap.servers'] = 'localhost:9092',
  }

  producer:send {
    topic = 'my.topic',
    payload = message:get_data(),
    -- how long to keep trying to submit to kafka
    -- before a lua error will be raised.
    -- This is the default.
    timeout = '1 minute',
  }
end)
```

See the [Kafka](../../reference/kumo.kafka/index.md) section of the Reference Manual for more information.
