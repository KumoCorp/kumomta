# Routing Messages via HTTP Request

Some sending environments use a mixture of different services to send messages, and while it's possible to relay messages through many services using SMTP, some services are only/better served via an HTTP API.

The following example shows how to send a queued message via custom lua, in this case assembling an API call and sending it to a third-party SMTP API relay provider.

!!!warning
    Storing credentials as hardcoded values in a policy script such as this is not recommended, instead, use the built-in Secrets Load function. See [https://docs.kumomta.com/reference/kumo.secrets/load/](https://docs.kumomta.com/reference/kumo.secrets/load/).

```lua
kumo.on('make.mailgun', function(domain, tenant, campaign)
  local client = kumo.http.build_client {}
  local sender = {}

  function sender:send(message)
    local request =
      client:post 'https://api.mailgun.net/v3/YOUR_DOMAIN_NAME/messages.mime'

    request:basic_auth('api', 'YOUR_API_KEY')
    request:form_multipart_data {
      to = message:recipient().email,
      message = {
        data = message:get_data(),
        file_name = 'mime.msg',
      },
    }

    -- Make the request
    local response = request:send()

    -- and handle the result
    local disposition = string.format(
      '%d %s %s',
      response:status_code(),
      response:status_reason(),
      response:text()
    )
    if response:status_is_success() then
      -- Success!
      return disposition
    end

    -- Failed!
    kumo.reject(400, disposition)
  end
  return sender
end)

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if tenant == 'mailgun-user' then
    return kumo.make_queue_config {
      protocol = {
        custom_lua = {
          constructor = 'make.mailgun',
        },
      },
    }
  end

  return kumo.make_queue_config {}
end)
```
