---
tags:
  - counter_series
---

# counter_series:increment

```lua
series:increment(value)
```

{{since('dev')}}

Adds `value` to the current bucket of the series.

* `value` — unsigned integer amount to add. Must be `>= 0`.

The current bucket is the one corresponding to the present moment in time;
older buckets rotate out automatically as time elapses.

Addition saturates at `u64::MAX`. To subtract from the current bucket, use
[delta](delta.md) with a negative value.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'deliveries.ok',
  num_buckets = 5,
  bucket_size = '1m',
}
series:increment(1)
```
