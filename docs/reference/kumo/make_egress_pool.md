# `kumo.make_egress_pool { PARAMS }`

Defines an *egress pool*, which is a collection of weighted *egress sources*
associated with the source of outbound traffic from the MTA.

A given scheduled queue can be associated with a pool and it will then use
*[Weighted Round
Robin](http://kb.linuxvirtualserver.org/wiki/Weighted_Round-Robin_Scheduling)*
(WRR) to distribute sends from that scheduled queue across the IPs contained
within its associated pool.  When a scheduled queue is idle for approximately
10 minutes, it will idle out and the round robin state will be reset for the
next send.

!!! info
    The *Weighted Round Robin* implementation in kumomta is considered to be
    **probabilistic**, achieving the configured distribution only when the rate
    of sending is sufficiently high (at least 1 message to a given site every
    few minutes), and is scoped per-*scheduled*-queue. There is no whole-machine
    nor whole-cluster coordination in the round robin implementation as those
    techniques introduce bottlenecks that limit scalability and are unnecessary
    at the kinds of volumes where it is important to implement distribution
    across sending IPs.

This function is intended to be used inside your
[get_egress_pool](../events/get_egress_pool.md) event handler.

`PARAMS` is a lua table which may have the following keys:

## name

Required string.

The name of the pool. This name can be referenced via
[make_queue_config().egress_pool](make_queue_config/egress_pool.md).

## entries

Required list of entries.

Each entry has a *name*, which must refer to a source that will be resolved via
your [get_egress_source](../events/get_egress_source.md) event, and an optional
weight:

```lua
local SOURCE_TO_IP = {
  ['ip-1'] = '10.0.0.1',
  ['ip-2'] = '10.0.0.2',
  ['ip-3'] = '10.0.0.3',
}

function setup_pools()
  return {
    ['BestReputation'] = kumo.make_egress_pool {
      name = 'BestReputation',
      entries = {
        { name = 'ip-1' },
      },
    },

    ['MediumReputation'] = kumo.make_egress_pool {
      name = 'MediumReputation',
      entries = {
        { name = 'ip-2', weight = 2 },
        -- we're warming up ip-3, so use it less frequently than ip-2
        { name = 'ip-3', weight = 1 },
      },
    },
  }
end

local POOLS = setup_pools()

kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    source_address = SOURCE_TO_IP[source_name],
  }
end)

kumo.on('get_egress_pool', function(pool_name)
  return POOLS[pool_name]
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

## ttl

Optional *time-to-live* specifying how long the pool definition should be
cached.  The cache has two purposes:

* To limit the number of configurations kept in memory at any one time
* To enable data to be refreshed from external storage, such as a json data
  file, or a database

The default TTL is 60 seconds, but you can specify any duration using a string
like `"5 mins"` to specify 5 minutes.

