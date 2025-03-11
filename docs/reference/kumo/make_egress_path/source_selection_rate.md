# source_selection_rate

{{since('dev')}}

Optional throttle spec string.

Specifies the maximum permitted rate at which the source that is
associated with this egress path can be selected during the promotion
of a message from the scheduled queue to the ready queue of this egress
path.

This option can be used to help manage IP-warmup.

For example, if you have two sources in a pool named `mypool`:

 * `new` - a new source that you want to use sparingly until you have established a good reputation for it
 * `established` - a source that is fully established

And you have a `shaping.toml` configuration like:

```toml
["example.com".sources."new"]
# You will probably want to set max_burst for this sort of throttle,
# to avoid clumping all the sends together at the start of the day
source_selection_rate = "100/day,max_burst=1"
```

and a `queue.toml` configuration like:

```toml
[queue.'example.com']
egress_pool = 'mypool'
```

then messages destined for `example.com` will use the regular weighted round-robin
source selection from `mypool`, but it will be modified slightly:

 * Whenever the `new` source is selected by the weighted round-robin
   algorithm, `source_selection_rate` (and `additional_source_selection_rates`)
   will be consulted to see if there are rate limits for selection.
 * If any selection rate limits are present, the throttles are sorted from
   smallest to largest and then checked
 * If there is remaining capacity in the throttles then the source will be selected.
 * The first limit that is exceeded will cause the source to be ineligible
   for this particular selection, and the round-robin algorithm will proceed
   to try the next potential candidate source

See also:

 * [additional_source_selection_rates](additional_source_selection_rates.md)
