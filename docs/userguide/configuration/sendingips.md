# Configuring Sending IPs

By default, all traffic injected to the KumoMTA server will be delivered using
the default interface configured on the host server. For smaller installations
this is acceptable, but best practices recommend separating mail streams into
their own IPs addresses in order to isolate reputation and enable larger
sending volumes than would be possible on a single IP address.

## Using the sources.lua Policy Helper

While the process for creating Egress Sources and Pools is defined below, most
users will want to take advantage of the `sources.lua` policy helper. This is a
supplemental script that takes care of the creation logic by leveraging a TOML
configuration file you define.

To use the `sources.lua` policy helper, add the following to your server policy
script:

```lua
-- Configure source IPs.
local sources = require 'policy-extras.sources'
sources:setup { '/opt/kumomta/etc/sources.toml' }
```

In addition, create a file at `/opt/kumomta/etc/sources.toml` and populate it
as follows:

```toml
[source."ip-1"]
source_address = "10.0.0.1"
ehlo_domain = 'mta1.examplecorp.com'

[source."ip-2"]
source_address = "10.0.0.2"
ehlo_domain = 'mta2.examplecorp.com'

[source."ip-3"]
source_address = "10.0.0.3"
ehlo_domain = 'mta3.examplecorp.com'

# Pool containing just ip-1, which has weight=1
[pool."pool-1"]
[pool."pool-1"."ip-1"]

# Pool with multiple ips
[pool."pool-2"]

[pool."pool-2"."ip-2"]
weight = 2

# We're warming up ip-3, so use it less frequently than ip-2
[pool."pool-2"."ip-3"]
weight = 1
```

The sources you define can include any options listed for the [make_egress_source](../../reference/kumo/make_egress_source.md) function.

## Assigning Messages to Pools

It's not enough to simply create an Egress Source and assign it to an Egress
Pool, the server requires explicit logic to know which message is assigned to
which Egress Pool.

This logic occurs in the events related to queue management, see the [Queue
Management](./queuemanagement.md#configuring-egress-pool-assignment) chapter
for more information.

## Provisioning Egress Sources Using Lua

!!!note
    Most users will be satisfied with using the policy helper shown above.
    This section and the remainder of this page is for more advanced users.

In KumoMTA, source IPs are described in an *Egress Source.* And Egress Source
represents an object that can be used to send messages and is not attached to a
particular protocol. While the most common use case is an IP address used for
SMTP, it could also define a specific outbound port for sending through
port-based NAT, or a specific configuration for sending over HTTP.

An Egress Source is defined using the **`kumo.make_egress_source`** function,
called during the init event. For more information, see the
[make_egress_source](../../reference/kumo/make_egress_source.md) chapter of the
Reference Manual.

By default, the only option required for defining an Egress Source is a name,
creating a logical grouping for messages used for queueing but still using the
default server IP address:

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    return kumo.make_egress_source {
      name = 'ip-1',
    }
  end
  error 'you need to do something for other source names'
end)
```

Typically an Egress source is used to assign messages to a specific IP address
for sending. It is a best practice for each source IP to have a unique hostname
used during the EHLO command, that matches a PTR record that points to the
external IP associated with the Egress Source. The IP address is set with
the *source* address option and the hostname is set using the *ehlo_domain*
option. The IP address used is not required to be unique to a given Egress
Source:

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1
    kumo.make_egress_source {
      name = 'ip-1',
      source_address = '10.0.0.1',
      ehlo_domain = 'mta1.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

KumoMTA supports both IPv4 and IPv6 for sending, based on the source address
assigned to the Egress Source:

```lua
kumo.on('get_egress_source', function(source_name)
  if source_name == 'ip-1' then
    -- Make a source that will emit from 10.0.0.1
    kumo.make_egress_source {
      name = 'ip-1',
      source_address = '2001:db8:3333:4444:5555:6666:7777:8888',
      ehlo_domain = 'mta2.examplecorp.com',
    }
  end
  error 'you need to do something for other source names'
end)
```

## Provisioning Egress Pools Using Lua

!!!note
    Most users will be satisfied with using the policy helper shown above.
    This section and the remainder of this page is for more advanced users.

Messages cannot be assigned directly to an Egress Source, but are instead
assigned to an Egress Pool. An Egress Pool contains one or more Egress Sources,
and messages assigned to the pool are assigned in a round-robin fashion by
default, with weighted round-robin available as an option.

A given Egress Source can be added to multiple Egress Pools.

Egress Pools are defined using the **`kumo.make_egress_pool`** function, called
during the `get_egress_pool` event:

```lua
-- Maps a source name to the corresponding IP address
local SOURCE_TO_IP = {
  ['ip-1'] = '10.0.0.1',
  ['ip-2'] = '10.0.0.2',
  ['ip-3'] = '10.0.0.3',
}

-- This makes it convenient to author the pools, but is not as efficient
-- as it could be. That is balanced out by using memoize below.
function setup_pools()
  local pools = {
    {
      name = 'BestReputation',
      entries = {
        { name = 'ip-1' },
      },
    },
    {
      name = 'MediumReputation',
      entries = {
        { name = 'ip-2', weight = 2 },
        -- we're warming up ip-3, so use it less frequently than ip-2
        { name = 'ip-3', weight = 1 },
      },
    },
  }
  local result = {}
  for _, pool in ipairs(pools) do
    result[pool.name] = kumo.make_egress_pool(pool)
  end
  return result
end

-- Wrap setup_pools as a caching version called get_pool_config
local get_pool_config = kumo.memoize(setup_pools, {
  name = 'setup-my-pools',
  ttl = '5 minutes',
  capacity = 10,
})

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    source_address = SOURCE_TO_IP[source_name],
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  local pools = get_pool_config()
  return pools[pool_name]
end)
```

For more information, see the
[make_egress_pool](../../reference/kumo/make_egress_pool.md) chapter of the
Reference Manual.

