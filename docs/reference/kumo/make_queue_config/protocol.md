# protocol

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

#### Specifying directory and file modes for maildir

{{since('dev')}}

If you are sharing the maildir with something like dovecot it can sometimes
be desirable to explicitly control the file permissions of the directory
structure and files that are created.  You can achieve this via the `dir_mode`
and `file_mode` parameters.

!!! note
    Lua doesn't support native octal literal numbers, so you must use
    `tonumber` as shown in the example below if you wish to specify
    the modes in octal

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  if domain == 'maildir.example.com' then
    return kumo.make_queue_config {
      protocol = {
        maildir_path = '/var/tmp/kumo-maildir',
        dir_mode = tonumber('775', 8),
        file_mode = tonumber('664', 8),
      },
    }
  end
end)
```

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

See [should_enqueue_log_record](../../events/should_enqueue_log_record.md) for
a more complete example.



