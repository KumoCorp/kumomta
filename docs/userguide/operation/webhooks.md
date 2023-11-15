# Publishing Log Events Via Webhooks

While logs are an invaluable resource for monitoring and troubleshooting mail
flows, log rotating and parsing adds complexity and latency when the goal is
loading the email event data into an existing platform.

Webhooks are ideal for near real-time integration into existing platforms,
providing the ability to send message events to a user-defined HTTP endpoint,
with queuing out of the box to ensure durability in the event of an error on
the part of the HTTP receiving service.

Webhooks are implemented in KumoMTA by triggering a Lua hook on log events that
allows for a policy script to load the log events into their own message queue
within the KumoMTA queueing structure like any other message, ensuring
durability and performance for queued log events.

Webhook events are moved through the queues like SMTP messages, and when they
enter the Ready Queue they are set to deliver via an arbitrary Lua event rather
than SMTP, with the Lua script configured to issue an HTTP request to the
destination server.

## Using the log_hooks.lua Helper

While the methods documented below can be used to implement advanced webhook delivery scenarios, most users will benefit from using the *log_hooks.lua* helper.

To implement the helper, add the following to your init.lua:

```lua
local log_hooks = require 'policy-extras.log_hooks'
log_hooks:new_json {
  name = 'webhook',
  -- log_parameters are combined with the name and
  -- passed through to kumo.configure_log_hook
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
  -- queue config are passed to kumo.make_queue_config.
  -- You can use these to override the retry parameters
  -- if you wish.
  -- The defaults are shown below.
  queue_config = {
    retry_interval = '1m',
    max_retry_interval = '20m',
  },
  -- The URL to POST the JSON to
  url = 'http://10.0.0.1:4242/log',
}
```

More advanced usage is possible by implementing the full call to the log_hooks.lua helper, in the following format:

```lua
local log_hooks = require 'policy-extras.log_hooks'
log_hooks:new {
  name = 'webhook',
  -- log_parameters are combined with the name and
  -- passed through to kumo.configure_log_hook
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
  -- queue config are passed to kumo.make_queue_config.
  -- You can use these to override the retry parameters
  -- if you wish.
  -- The defaults are shown below.
  queue_config = {
    retry_interval = '1m',
    max_retry_interval = '20m',
  },
  constructor = function(domain, tenant, campaign)
    local connection = {}
    local client = kumo.http.build_client {}
    function connection:send(message)
      local response = client
        :post('http://10.0.0.1:4242/log')
        :header('Content-Type', 'application/json')
        :body(message:get_data())
        :send()

      local disposition = string.format(
        '%d %s: %s',
        response:status_code(),
        response:status_reason(),
        response:text()
      )

      if response:status_is_success() then
        return disposition
      end

      -- Signal that the webhook request failed.
      -- In this case the 500 status prevents us from retrying
      -- the webhook call again, but you could be more sophisticated
      -- and analyze the disposition to determine if retrying it
      -- would be useful and generate a 400 status instead.
      -- In that case, the message we be retryed later, until
      -- it reached it expiration.
      kumo.reject(500, disposition)
    end
    return connection
  end,
}
```

## Configuring a Log Hook

The first step in setting up Webhooks is to turn on the log hook. This adds a
Lua event for every log entry so that a script can be implemented to
selectively queue the event data:

The call to `configure_log_hook` is placed in the init event handler:

```lua
kumo.on('init', function()
  kumo.configure_log_hook {
    name = 'webhook',
    headers = { 'Subject', 'X-Customer-ID' },
  }
end)
```

The `configure_log_hook` function can take similar parameters to the
`configure_local_logs` function with regards to additional data and formatting,
see the
[configure_log_hook](../../reference/kumo/configure_log_hook.md)
page of the Reference manual for more information.

## Handling Log Hook Messages

With the `configure_log_hook` call added to the init event, the KumoMTA server
creates a new message object for each log entry, specially formatted to contain
the log record as the message body.

The message will be passed to the
[should_enqueue_log_record](../../reference/events/should_enqueue_log_record.md)
event, which is where we can add logic to process the message and queue it for
later delivery.

The following example shows how to handle the event, and how to avoid a loop
that can occur if the webhook log events are in turn processed as webhooks:

```lua
kumo.on('should_enqueue_log_record', function(msg)
  local log_record = msg:get_meta 'log_record'
  -- avoid an infinite loop caused by logging that we logged that we logged...
  -- Check the log record: if the record was destined for the webhook queue
  -- then it was a record of the webhook delivery attempt and we must not
  -- log its outcome via the webhook.
  if log_record.queue ~= 'webhook' then
    -- was some other event that we want to log via the webhook
    msg:set_meta('queue', 'webhook')
    return true
  end
  return false
end)
```

The preceding example assigns the messages to a queue named `webhook` if the
message is not already associated with that queue (a record of a webhook
delivery event), and otherwise returns false, indicating that the record should
not be queued. See the
[should_enqueue_log_record](../../reference/events/should_enqueue_log_record.md)
page of the Reference Manual for more information.

## Configuring A Queue Handler for Webhooks

When a message is ready to be queued, the
[get_queue_config](../../reference/events/get_queue_config.md) event is fired,
at which point we can specify the protocol of the queue, in this case
`custom_lua`. In the example below, we check whether the message is queued to
the `webhook` queue and act accordingly:


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
```

For more information on configuring protocols, see the
[get_queue_config](../../reference/kumo/make_queue_config.md) section of the
Reference Manual.

## Sending Messages via HTTP

With the `custom_lua` protocol defined and a custom event trigger declared, the
next step is to catch the `make.webhook` event with code that sends the message
contents over HTTP.

The following example sends the content of the webhook queued message over HTTP
to a configured host as a POST:

```lua
-- This is a user-defined event that matches up to the custom_lua
-- constructor used in `get_queue_config` below.
-- It returns a lua connection object that can be used to "send"
-- messages to their destination.
kumo.on('make.webhook', function(domain, tenant, campaign)
  local connection = {}
  local client = kumo.http.build_client {}
  function connection:send(message)
    local response = client
      :post(string.format('http://127.0.0.1:%d/log', WEBHOOK_PORT))
      :header('Content-Type', 'application/json')
      :body(message:get_data())
      :send()

    local disposition = string.format(
      '%d %s: %s',
      response:status_code(),
      response:status_reason(),
      response:text()
    )

    if response:status_is_success() then
      return disposition
    end

    -- Signal that the webhook request failed.
    -- In this case the 500 status prevents us from retrying
    -- the webhook call again, but you could be more sophisticated
    -- and analyze the disposition to determine if retrying it
    -- would be useful and generate a 400 status instead.
    -- In that case, the message we be retryed later, until
    -- it reached it expiration.
    kumo.reject(500, disposition)
  end
  return connection
end)
```

!!!warning
    Storing credentials as hardcoded values in a policy script such as this is
    not recommended, instead, use the built-in Secrets Load function. See
    [kumo.secrets/load/](../..//reference/kumo.secrets/load.md).

This same methodology could also be used to deliver queued SMTP messages to a
third-party API, see the [Routing Messages via HTTP Request](../policy/http.md)
page of the Policy chapter for more information.

This same methodology could also be used to deliver log events and queued
messages via AMQP, see the [Routing Messages via AMQP](../policy/amqp.md) page
of the Policy chapter for more information.
