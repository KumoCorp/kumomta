# `kumo.make_egress_source {PARAMS}`

Defines an *egress source*, which is an entity associated with the source of
outbound traffic from the MTA.  A source must be referenced by a
[pool](../make_egress_pool/index.md) to be useful.

This function is intended to be used inside your
[get_egress_source](../../events/get_egress_source.md) event handler.

A source must have at a minimum a *name*, which will be used in logging/reporting.

`PARAMS` is a lua table which may have the following keys:

