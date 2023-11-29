# `kumo.on('should_enqueue_log_record', function(message, hook_name))`

This event is triggered when
[kumo.configure_log_hook](../kumo/configure_log_hook.md) has been used to
enable it.

{{since('2023.08.22-4d895015', indent=True)}}
    The *hook_name* parameter was added. It corresponds to the name field
    that was passed to `kumo.configure_log_hook` and is present to allow
    you to decide whether a given message should get queued for a given
    hook instance.

When enabled, each log record will generate a new
[Message](../message/index.md) object with the following attributes:

* Sender will be set to the sender of the originating message
* Recipient will be set to the recipient of the originating message
* the `log_record` meta value will be set to the
  [Log Record](../kumo/configure_local_logs.md#log-record) that is being logged
* The message body/data will be set to the textual representation of the log
  record. By default this will be the JSON serialization of the log record, but
  if you have enabled templating using the `per_record` parameter to
  [kumo.configure_log_hook](../kumo/configure_log_hook.md) then it will be
  whatever your template evaluated to for that record.
* The `reception_protocol` meta value will be set to `"LogRecord"`

That log message is passed to the `should_enqueue_log_record` hook, which must return
a boolean value that indicates whether the log message should be queued and acted upon
(return true), or whether the log message should be discarded.

```lua
kumo.on('should_enqueue_log_record', function(msg, hook_name)
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

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'webhook' then
    -- Use the `make.webhook` event to handle delivery
    -- of webhook log records
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          constructor = 'make.webhook',
        },
      },
    }
  end
  return kumo.make_queue_config {}
end)
```

{{since('2023.11.28-b5252a41', indent=True)}}
    It is now possible to use `kumo.on` to register multiple handlers for
    this event.  The handlers will be called in the order that they were
    registered.  If a handler returns `nil` then the next handler will be
    called. Conversely, if a handler returns either `true` or `false`,
    its return value is taken as the definitive outcome and no further handlers
    will be called.

    This behavior is intended to make it easier to compose multiple helpers
    or lua modules together.
