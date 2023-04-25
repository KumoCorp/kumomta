# Configuring Queue Management

After a message is injected, it is placed into a Scheduled Queue based on the combination of its Campaign, Tenant, and Destination Domain. If any of these attributes are not set, the queue will be based on whichever elements are present. The Scheduled Queue is also used for messages that encountered a temporary failure and are awaiting a retry. See [Configuration Concepts](./concepts.md) for more information.

## Configuring Message Life and Retry Times

There is no throttling configured at the Scheduled Queue level, instead, the Scheduled Queue is where messages are evaluated when retries are needed, meaning that at the Scheduled Queue level we configure settings such as the time between retries and the maximum age of a message.

The settings for retry interval and message age are typically set globally and then overridden on a per-tenant basis.

In the example below, a collection of per-tenant parameters is created, with global parameters set separately. When the `get_queue_config` event fires, the two collections are merged and the resulting collection of parameters is passed to the `kumo.make_queue_config` function and passed back to the event handler. See the [make_queue_config](../../reference/kumo/make_queue_config.md) page of the Reference Manual for more information.

While the event includes arguments for the destination domain, tenant, and campaign, this example is based on the assumption that queue configuration is only customized at the tenant level:

```lua
local TENANT_PARAMS = {
  TenantOne = {
    max_age = '5 minutes',
  },
}

kumo.on('get_queue_config', function(domain, tenant, campaign)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
  }
  merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

Note that the example above is designed specifically to show one method of storing and managing the parameters of the `kumo.make_queue_config` function, but users are free to store and combine parameters as they see fit.

## Configuring Egress Pool Assignment

It's not enough to configure an Egress Pool, the server must also have assignment logic to determine which Egress pool should be used for a given message.

Any logic can be used for Egress Pool assignment, leveraging the domain, tenant, and campaign provided for the `get_queue_config` event. This example is based on the idea that the Egress Pool will be named after the message's tenant:

```lua
local TENANT_PARAMS = {
  TenantOne = {
    max_age = '5 minutes',
  },
}

kumo.on('get_queue_config', function(domain, tenant, campaign)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
    egress_pool = tenant,
  }
  merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

An example of assigning a tenant name to a message is as follows, occurring during the `smtp_server_message_received` event, in this case using the tenant name stored in a header called **X-Tenant:**

```lua
kumo.on('smtp_server_message_received', function(msg)
  -- Assign tenant based on X-Tenant header.
  local tenant = msg:get_first_named_header_value 'X-Tenant'
  if tenant then
    msg:set_meta('tenant', tenant)
  end
end)
```

Note that the example above does not have any handling for an empty or incorrect **X-Tenant** header.
