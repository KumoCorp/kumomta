# selection_strategy

{{since('dev')}}

Optional string. Controls how a source is chosen from the pool each time a
message moves from its scheduled queue into a ready queue.

The possible values are:

* `"WeightedRoundRobin"` - the default. Every selection uses *Weighted Round
  Robin* across the [entries](entries.md), independent of any prior selection
  for the same message.

* `"StickyWeightedRoundRobin"` - the first selection for a message uses
  *Weighted Round Robin* and the chosen source name is recorded in the
  message metadata as `sticky_egress_source`. Later selections for that
  message reuse the recorded source for as long as it remains an entry in
  the pool with a non-zero weight.

  If the recorded source is no longer in the pool, a fresh selection is made
  and recorded. This can happen when the pool definition changes or the
  message is rebound to a queue that uses a different pool.

  The recorded source is cleared when a message is transferred to another
  node via `kumo.xfer`.
