# Configuring Sending IPs

By default, all traffic injected to the KumoMTA server will be delivered using the default interface configured on the host server. For smaller installations this is acceptable, but best practices recommend separating mail streams into their own IPs addresses in order to isolate reputation and enable larger sending volumes than would be possible on a single IP address.

## Defining an Egress Source

In KumoMTA, source IPs are described in an *Egress Source.* And Egress Source represents an object that can be used to send messages, and is not attached to a particular protocol. While the most common use case is an IP address used for SMTP, it could also define a specific outbound port for sending through port-based NAT, or a specific configuration for sending over HTTP.

An Egress Source is defined using the **`kumo.define_egress_source`** function, called during the init event. For more information, see the [define_egress_source](../../reference/kumo/define_egress_source.md) chapter of the Reference Manual.

By default, the only option required for defining an Egress Source is a name, creating a logical grouping for messages used for queueing but still using the default server IP address:

```lua
kumo.on('init', function()
  -- Make a source that just has a name, but otherwise doesn't
  -- specify any particular source configuration
  kumo.define_egress_source {
    name = 'example',
  }
end)
```

Typically an Egress source is used to assign messages to a specific IP address for sending. It is a best practice for each source IP to have a unique hostname used during the EHLO command, that matches a PTR record that points to the external IP associated with the Egress Source. The IP address is set with the *source* address option and the hostname is set using the *ehlo_domain* option. The IP address used is not required to be unique to a given Egress Source:

```lua
kumo.on('init', function()
  -- Make a source that will emit from 10.0.0.1
  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
    ehlo_domain = 'mta1.examplecorp.com',
  }
end)
```

KumoMTA supports both IPv4 and IPv6 for sending, based on the source address assigned to the Egress Source:

```lua
kumo.on('init', function()
  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '2001:db8:3333:4444:5555:6666:7777:8888',
    ehlo_domain = 'mta2.examplecorp.com',
  }
end)
```

## Defining an Egress Pool

Messages cannot be assigned directly to an Egress Source, but are instead assigned to an Egress Pool. An Egress Pool contains one or more Egress Sources, and messages assigned to the pool are assigned in a round-robin fashion by default, with weighted round-robin available as an option.

A given Egress Source can be added to multiple Egress Pools.

Egress Pools are defined using the **`kumo.define_egress_pool`** function, called during the init event:

```lua
kumo.on('init', function()
  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
    ehlo_domain = 'mta1.examplecorp.com',
  }
  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '2001:db8:3333:4444:5555:6666:7777:8888',
    ehlo_domain = 'mta2.examplecorp.com',
  }

  kumo.define_egress_pool {
    name = 'CustomerA',
    entries = {
      { name = 'ip-1' },
    },
  }

  kumo.define_egress_pool {
    name = 'SharedPool',
    entries = {
      { name = 'ip-1', weight = 1 },
      { name = 'ip-2', weight = 2 },
    },
  }
end)
```

For more information, see the [define_egress_pool](../../reference/kumo/define_egress_pool.md) chapter of the Reference Manual.

## Assigning Messages to Pools

It's not enough to simply create an Egress Source and assign it to an Egress Pool, the server requires explicit logic to know which message is assigned to which Egress Pool.

This logic occurs in the events related to queue management, see the [Queue Management](./queuemanagement.md#configuring-egress-pool-assignment) chapter for more informaton.
