# timeq_pop_tardiness

```
Type: Histogram
Buckets: 0.25, 0.5, 1.0, 2.5, 3.0, 5.0, 10.0, 15.0
```
The time difference between the due and current time for a singleon timerwheel pop.


This gives an indication of whether the scheduled queue
maintainer is keeping up with the load.  It is generally
acceptable for this value to be a few seconds "late" due
to a combination of time wheel bucket granularity and
overall scheduling priority.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `timeq_pop_tardiness_count` - a counter tracking how many events have been accumulated into the histogram
  * `timeq_pop_tardiness_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `timeq_pop_tardiness_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.25"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `timeq_pop_tardiness_bucket`.

While it is possible to calculate a mean average for `timeq_pop_tardiness` by computing `timeq_pop_tardiness_sum / timeq_pop_tardiness_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
