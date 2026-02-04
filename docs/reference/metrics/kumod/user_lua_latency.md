# user_lua_latency

```
Type: Histogram
Labels: label
Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
```
How long something user-defined took to run in your lua policy.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `user_lua_latency_count` - a counter tracking how many events have been accumulated into the histogram
  * `user_lua_latency_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `user_lua_latency_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.005"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `user_lua_latency_bucket`.

While it is possible to calculate a mean average for `user_lua_latency` by computing `user_lua_latency_sum / user_lua_latency_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
