# message_save_latency

```
Type: Histogram
Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
```
how long it takes to save a message to spool.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `message_save_latency_count` - a counter tracking how many events have been accumulated into the histogram
  * `message_save_latency_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `message_save_latency_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.005"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `message_save_latency_bucket`.

While it is possible to calculate a mean average for `message_save_latency` by computing `message_save_latency_sum / message_save_latency_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
