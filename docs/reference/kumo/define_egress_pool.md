# `kumo.define_egress_pool { PARAMS }`

Defines an *egress pool*, which is a collection of weighted *egress sources*
associated with the source of outbound traffic from the MTA.

This function should be called only from inside your [init](../events/init.md)
event handler.

`PARAMS` is a lua table which may have the following keys:

## name

Required string.

The name of the pool. This name can be referenced via
[make_queue_config().egress_pool](make_queue_config.md#egress_pool).

## entries

Required list of entries.

Each entry has a *name*, which must refer to an source that has been defined via
[define_egress_source](define_egress_source.md), and an optional weight:

```lua
kumo.on('init', function()
  kumo.define_egress_source {
    name = 'ip-1',
    source_address = '10.0.0.1',
  }
  kumo.define_egress_source {
    name = 'ip-2',
    source_address = '10.0.0.2',
  }
  kumo.define_egress_source {
    name = 'ip-3',
    source_address = '10.0.0.3',
  }

  kumo.define_egress_pool {
    name = 'BestReputation',
    entries = {
      { name = 'ip-1' },
    },
  }

  kumo.define_egress_pool {
    name = 'MediumReputation',
    entries = {
      { name = 'ip-2', weight = 2 },
      -- we're warming up ip-3, so use it less frequently than ip-2
      { name = 'ip-3', weight = 1 },
    },
  }
end)
```

The weight is used as part of [Weighted
Round-Robin](http://kb.linuxvirtualserver.org/wiki/Weighted_Round-Robin_Scheduling)
selection for the source from the pool.

If the weights are all equal, or are all left unspecified, then simple round-robin
selection of sources will occur.

Otherwise, the weight influences how often a given source will be used for traffic
originating from this pool.

A weight of `0` is permitted: it is equivalent to not including the associated
sources in the list of entries.

If weight is left unspecified, it defaults to `1`.
