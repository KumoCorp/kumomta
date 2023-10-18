# Routing Messages via AMQP

In addition to local logging and Webhooks, KumoMTA can relay log events (or other queued messages) via AMQP.

KumoMTA supports publishing via AMQP, using Lua.

The process to queue log events and make them available for sending via `custom_lua` as a protocol is covered in the [Publishing Log Events Via Webhooks](../operation/webhooks.md) section of the Operations chapter of the User Guide.

## Configuring A Queue Handler for AMQP

When a message is ready to be queued, the `get_queue_config` event is fired, at which point we can specify the protocol of the queue, in this case `custom_lua`. In the example below, we check whether the message is queued to the `amqp` queue and act accordingly:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'amqp' then
    -- Use the `make.webhook` event to handle delivery
    -- of webhook log records
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          -- this will cause an event called `make.webhook` to trigger.
          -- You can pick any name for this event, so long as it doesn't
          -- collide with a pre-defined event, and so long as you bind
          -- to it with a kumo.on call
          constructor = 'make.amqp',
        },
      },
    }
  end
  return kumo.make_queue_config {}
end)
```

## Sending Messages via AMQP

With the custom_lua protocol defined and a custom event trigger declared, the next step is to catch the `make.amqp` event with code that sends the message contents over HTTP.

The following example sends the content of the log message via AMQP:

```lua
-- This is a user-defined event that matches up to the custom_lua
-- constructor used in `get_queue_config` below.
-- It returns a lua connection object that can be used to "send"
-- messages to their destination.
kumo.on('make.amqp', function(domain, tenant, campaign)
  local client = kumo.amqp.build_client 'amqp://localhost'
  local confirm = client:publish {
    routing_key = 'logging',
    payload = message:get_data(),
  }
  local result = confirm:wait()

  if result.status == 'Ack' or result.status == 'NotRequested' then
    return
  end
  -- result.status must be `Nack`; log the full result
  kumo.reject(500, kumo.json_encode(result))
end)
```

See the [AMQP](../../reference/kumo.amqp/index.md) section of the Reference Manual for more information.
