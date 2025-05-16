# kumo.make_egress_pool

```lua
kumo.make_egress_pool { PARAMS }
```

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
[get_egress_pool](../../events/get_egress_pool.md) event handler.

`PARAMS` is a lua table which may have the following keys:

## Egress Pool Parameters { data-search-exclude }
