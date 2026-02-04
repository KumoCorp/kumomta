# dkim_signer_key_fetch

```
Type: Histogram
Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
```
How long it takes to obtain a dkim key.


This measures that time that it takes to load dkim
private keys from whatever storage medium is configured.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `dkim_signer_key_fetch_count` - a counter tracking how many events have been accumulated into the histogram
  * `dkim_signer_key_fetch_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `dkim_signer_key_fetch_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.005"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `dkim_signer_key_fetch_bucket`.

While it is possible to calculate a mean average for `dkim_signer_key_fetch` by computing `dkim_signer_key_fetch_sum / dkim_signer_key_fetch_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
