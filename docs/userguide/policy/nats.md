# Routing Messages via NATS
{{since('dev')}}

In addition to local logging and Webhooks, KumoMTA can relay log events (or other queued messages) via [NATS JetStream](https://nats.io/).

KumoMTA supports publishing via NATS, using Lua.

The process to queue log events and make them available for sending via `custom_lua` as a protocol is covered in the [Publishing Log Events Via Webhooks](../operation/webhooks.md) section of the Operations chapter of the User Guide.

## Configuring A Queue Handler for NATS

When a message is ready to be queued, the `get_queue_config` event is fired, at which point we can specify the protocol of the queue, in this case, `custom_lua`. In the example below, we check whether the message is queued to the `nats` queue and acts accordingly:

```lua
kumo.on('init', function()
  nats = kumo.nats.connect {
    servers = {'127.0.0.1:4222'},
  }
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'nats' then
    -- Use the `make.webhook` event to handle delivery
    -- of webhook log records
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          -- this will cause an event called `make.webhook` to trigger.
          -- You can pick any name for this event, so long as it doesn't
          -- collide with a pre-defined event, and so long as you bind
          -- to it with a kumo.on call
          constructor = 'make.nats',
        },
      },
    }
  end
  return kumo.make_queue_config {}
end)
```

## Sending Messages via NATS

With the custom_lua protocol defined and a custom event trigger declared, the next step is to catch the `make.nats` event with code that sends the message contents over HTTP.

The following example publishes the content of the log message via NATS:

```lua
-- This is a user-defined event that matches up to the custom_lua
-- constructor used in `get_queue_config` below.
-- The connection must be established before in order to "publish"
-- messages to their destination.
kumo.on('make.nats', function(domain, tenant, campaign)
  nats:publish {
    subject = 'subject',
    payload = message:get_data(),
  }
end)
```

See the [NATS](../../reference/kumo.nats/index.md) section of the Reference Manual for more information.
