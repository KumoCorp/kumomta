# Configuring Queue Management

After a message is injected, it is placed into a Scheduled Queue based on the
combination of its Campaign, Tenant, Destination and Routing Domains. If any of
these attributes are not set, the queue will be based on whichever elements are
present. The Scheduled Queue is also used for messages that encountered a
temporary failure and are awaiting a retry. See [Configuration
Concepts](./concepts.md) for more information.

## Using The Queues Helper

To help simplify configuration for those with typical use cases, we have provided the *queue.lua* policy helper.

The *queue.lua* policy helper simplifies configuration of queue management, including identifying and assigning tenant and campaign information as well as message scheduling.

To use the *queue.lua* policy helper, adding the following to your *init.lua* policy:

```lua
local queue_module = require 'policy-extras.queue'
local queue_helper =
  queue_module:setup { '/opt/kumomta/etc/policy/queues.toml' }
```

In addition, add a call to perform queue management via the `queue_helper:apply(msg)` method to your incoming message events:

```lua
-- Processing of incoming messages via SMTP
kumo.on('smtp_server_message_received', function(msg)
  -- Call the queue helper to set up the queue for the message.
  queue_helper:apply(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)

-- Processing of incoming messages via HTTP
kumo.on('http_message_generated', function(msg)
  -- Call the queue helper to set up the queue for the message.
  queue_helper:apply(msg)
  -- SIGNING MUST COME LAST OR YOU COULD BREAK YOUR DKIM SIGNATURES
  dkim_signer(msg)
end)
```

In addition, create a file at `/opt/kumomta/etc/queues.toml` and populate it
as follows:

{% call toml_data() %}
# Allow optional scheduled sends based on this header
# https://docs.kumomta.com/reference/message/import_scheduling_header
scheduling_header = "X-Schedule"

# Set the tenant from this header and delete the header for added security
tenant_header = "X-Tenant"
remove_tenant_header = true

# Set the campaign from this header and delete the header for added security
campaign_header = "X-Campaign"
remove_campaign_header = true

# The tenant to use if no tenant_header is present
default_tenant = "default-tenant"

[tenant.'default-tenant']
egress_pool = 'pool-1'

[tenant.'mytenant']
# Which pool should be used for this tenant
egress_pool = 'pool-2'
# Override maximum message age based on tenant; this overrides settings at the domain level
max_age = '10 hours'

# Only the authorized identities are allowed to use this tenant via the tenant_header
#require_authz = ["daniel"]

# The default set of parameters
[queue.default]
max_age = '24 hours'

# If you configure a domain for `relay_to = true` in the listener_domains
# helper, you will need to configure a routing domain so that KumoMTA knows
# where to relay the messages to internally, since an MX lookup will return
# the IP for your KumoMTA instance:
[queue.'my.own.hostname']
routing_domain = '[10.0.0.1]'

# Base settings for a given destination domain.
# These are overridden by more specific settings
# in a tenant or more specific queue
[queue.'gmail.com']
max_age = '22 hours'
retry_interval = '17 mins'

[queue.'gmail.com'.'mytenant']
# options here for domain=gmail.com AND tenant=mytenant for any unmatched campaign

[queue.'gmail.com'.'mytenant'.'welcome-campaign']
# options here for domain=gmail.com, tenant=mytenant, and campaign='welcome-campaign'
{% endcall %}

## Configuring Message Life and Retry Times Using Lua

There is no throttling configured at the Scheduled Queue level, instead, the
Scheduled Queue is where messages are evaluated when retries are needed,
meaning that at the Scheduled Queue level we configure settings such as the
time between retries and the maximum age of a message.

The settings for retry interval and message age are typically set globally and
then overridden on a per-tenant basis.

In the example below, a collection of per-tenant parameters is created, with
global parameters set separately. When the
[get_queue_config](../../reference/events/get_queue_config.md)  event fires,
the two collections are merged and the resulting collection of parameters is
passed to the `kumo.make_queue_config` function and passed back to the event
handler.  See the
[make_queue_config](../../reference/kumo/make_queue_config/index.md) page of the
Reference Manual for more information.

While the event includes arguments for the destination domain, tenant, and
campaign, this example is based on the assumption that queue configuration is
only customized at the tenant level:

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local TENANT_PARAMS = {
  TenantOne = {
    max_age = '5 minutes',
  },
}

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
  }
  utils.merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

Note that the example above is designed specifically to show one method of
storing and managing the parameters of the
[kumo.make_queue_config](../../reference/kumo/make_queue_config/index.md) function,
but users are free to store and combine parameters as they see fit.

## Configuring Egress Pool Assignment Using Lua

It's not enough to configure an Egress Pool, the server must also have
assignment logic to determine which Egress pool should be used for a given
message.

Any logic can be used for Egress Pool assignment, leveraging the domain,
tenant, and campaign provided for the
[get_queue_config](../../reference/events/get_queue_config.md) event. This
example is based on the idea that the Egress Pool will be named after the
message's tenant:

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

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
    -- Here we are assuming that there is an egress_pool configured
    -- for each valid tenant. If tenant is nil then the built-in
    -- "unspecified" egress pool will be used.
    egress_pool = tenant,
  }
  utils.merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

An example of assigning a tenant name to a message is as follows, occurring
during the
[smtp_server_message_received](../../reference/events/smtp_server_message_received.md)
event, in this case using the tenant name stored in a header called
`X-Tenant:`

```lua
kumo.on('smtp_server_message_received', function(msg)
  -- Assign tenant based on X-Tenant header.
  local tenant = msg:get_first_named_header_value 'X-Tenant'
  if tenant then
    msg:set_meta('tenant', tenant)
  end
end)
```

Note that the example above does not have any handling for an empty or
incorrect **X-Tenant** header.

## Throttling The Scheduled Queue

By default KumoMTA moves messages from the Scheduled Queue to the Ready Queue as quickly there is room available in the Ready Queue, with the rate impacted only by how quickly the Ready queues have available space as limited by traffic shaping throttles or sending reputation.

Under certain circumstances it can be beneficial to throttle the flow of messages from the Scheduled Queue to the Ready Queue. Some examples of such use cases include:

* Throttling a campaign that is expected to result in increased call center load.
* Throttling a tenant that is new and part of a shared IP pool to limit the impact of potential abuse.
* Throttling a tenant based on per-domain feedback from a Mailbox Provider (MBP).

By using the [max_message_rate](../../reference/kumo/make_queue_config/max_message_rate.md) option in the queues helper you can define the specific throttle to use for a given Scheduled Queue scope.

{% call toml_data() %}
[queue.'gmail.com'.'mytenant']
max_age = '24 hours'
max_message_rate = '100/s'
{% endcall %}

Note that the `max_message_rate` option applies on a per-queue basis even when configured for a less specific scope, where a queue is defined as campaign@tenant:domain. This means that if you set the `max_message_rate` option for a given tenant, it does **not** limit the tenant to a given rate, it limits *every created queue for that tenant* to the specified rate.

To limit the *collective* set of queues for a given scope use the `overall_max_message_rate` option. This will enforce the limit across all Scheduled queues for the defined scope:

{% call toml_data() %}
[tenant.'mytenant']
max_age = '24 hours'
overall_max_message_rate = '100/s'
{% endcall %}

In this example the listed `mytenant` tenant will be throttled to 100 messages per second across all Scheduled queues, or in other words regardless of campaign or destination domain.
