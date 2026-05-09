---
tags:
  - counter_series
---

# counter_series:sum_over

```lua
series:sum_over(duration)
```

{{since('dev')}}

Returns the rolling total over the requested time span.

* `duration` — span to sum over. Accepts an integer number of seconds, or a
  duration string such as `"30s"`, `"5m"`.

The duration is rounded **up** to a whole number of buckets, so a span
shorter than `bucket_size` still includes the current bucket. The
effective span is also capped at `num_buckets * bucket_size`; requesting a
longer span is equivalent to calling [sum](sum.md).

Calling `sum_over` also rotates the ring buffer, so any buckets that have
aged out since the last access are zeroed before the result is computed.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'deliveries.ok',
  num_buckets = 5,
  bucket_size = '1m',
}
print('last 2 minutes:', series:sum_over '2m')
```
