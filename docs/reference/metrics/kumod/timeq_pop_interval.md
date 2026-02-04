# timeq_pop_interval

```
Type: Histogram
Buckets: 3.0, 4.0, 5.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0
```
The amount of time that passes between calls to TimeQ::pop.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `timeq_pop_interval_count` - a counter tracking how many events have been accumulated into the histogram
  * `timeq_pop_interval_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `timeq_pop_interval_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="3.0"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `timeq_pop_interval_bucket`.

While it is possible to calculate a mean average for `timeq_pop_interval` by computing `timeq_pop_interval_sum / timeq_pop_interval_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
