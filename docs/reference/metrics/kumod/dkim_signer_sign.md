# dkim_signer_sign

```
Type: Histogram
Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
```
how long it takes to dkim sign parsed messages.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `dkim_signer_sign_count` - a counter tracking how many events have been accumulated into the histogram
  * `dkim_signer_sign_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `dkim_signer_sign_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.005"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `dkim_signer_sign_bucket`.

While it is possible to calculate a mean average for `dkim_signer_sign` by computing `dkim_signer_sign_sum / dkim_signer_sign_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
