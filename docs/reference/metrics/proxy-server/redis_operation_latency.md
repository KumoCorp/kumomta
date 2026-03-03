# redis_operation_latency

```
Type: Histogram
Labels: service, operation, status
Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
```
The latency of an operation talking to Redis.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

The `service` key represents the redis server/service. It is not
a direct match to a server name as it is really a hash of the
overall redis configuration information used in the client.
It might look something like:
`redis://127.0.0.1:24419,redis://127.0.0.1:7779,redis://127.0.0.1:29469-2ce79dd1`
for a cluster configuration, or `redis://127.0.0.1:16267-f4da6e64`
for a single node cluster configuration.
You should anticipate that the `-HEX` suffix can and will change
in an unspecified way as you vary the redis connection parameters.

The `operation` key indicates the operation, which can be a `ping`,
a `query` or a `script`.

`status` will be either `ok` or `error` to indicate whether this
is tracking a successful or failed operation.

Since histograms track a count of operations, you can track the
rate of `redis_operation_latency_count` where `status=error`
to have an indication of the failure rate of redis operations.


## Histogram
This metric is a histogram which means that it is exported as three underlying metrics:

  * `redis_operation_latency_count` - a counter tracking how many events have been accumulated into the histogram
  * `redis_operation_latency_sum` - a counter tracking the total value of all of the events have been accumulated into the histogram
  * `redis_operation_latency_bucket` - a counter tracking the number of events that fall within the various buckets shown above.  This counter has an additional `le` label that indicates the bucket threshold.  For example, the first bucket for this histogram will generate a label `le="0.005"` which will keep track of the number of events whose value was *less-or-equal* (le) that value.

The recommended visualization for a histogram is a heatmap based on `redis_operation_latency_bucket`.

While it is possible to calculate a mean average for `redis_operation_latency` by computing `redis_operation_latency_sum / redis_operation_latency_count`, it can be difficult to reason about what that value means if the traffic patterns are not uniform since the launch of the process.  We strongly recommend using a heatmap visualization instead of computing an average value.
