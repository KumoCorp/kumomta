---
tags:
  - counter_series
---

# counter_series:sum

```lua
series:sum()
```

{{since('2026.05.12-a6845223')}}

Returns the rolling total across every bucket in the series — that is, the
sum over the full retention span of `num_buckets * bucket_size`.

Use [sum_over](sum_over.md) to total a shorter span.

Calling `sum` also rotates the ring buffer, so any buckets that have aged
out since the last access are zeroed before the result is computed.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'deliveries.ok',
  num_buckets = 5,
  bucket_size = '1m',
}
series:increment(3)
print('total over the last 5 minutes:', series:sum())
```
