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

We strongly recommend that all users make use of the `policy-extras.log_hooks`
module for their web (or other protocol) hooks. The module is much more
convenient to use than the underlying low level events, and handles some
subtle edge cases for you.

To implement the helper, add the following to your init.lua:

```lua
local log_hooks = require 'policy-extras.log_hooks'

-- Send a JSON webhook to a local network host.
-- See https://docs.kumomta.com/userguide/operation/webhooks/
log_hooks:new_json {
  name = 'webhook',
  url = 'http://10.0.0.1:4242/log',
  log_parameters = {
    headers = { 'Subject', 'X-Customer-ID' },
  },
}
```

!!!Warning
    The call to `new_json` must appear before the queues helper for it to work
    properly. See the [Example Config](../configuration/example.md) to see a
    working layout for the `init.lua` file.

More advanced usage is possible by implementing the full call to the
`log_hooks.lua` helper; the example below shows approximately
how you might define your own equivalent of `log_hooks:new_json`:

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

  -- The constructor is called when kumod needs to initiate
  -- a new connection to the log target. It must return
  -- a connection object
  constructor = function(domain, tenant, campaign)
    -- Define the connection object
    local connection = {}

    -- Create an HTTP client
    local client = kumo.http.build_client {}

    -- The send method is called for each log event
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

    -- The close method is called when the connection needs
    -- to be closed
    function connection:close()
      client:close()
    end

    return connection
  end,
}
```

You can use the above to define logging that uses other protocols
than HTTP, such as AMQP or Kafka.

