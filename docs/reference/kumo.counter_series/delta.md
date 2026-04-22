---
tags:
  - counter_series
---

# counter_series:delta

```lua
series:delta(value)
```

{{since('dev')}}

Adjusts the current bucket of the series by a signed amount.

* `value` — signed integer delta. Positive values add; negative values
  subtract.

Bucket values are unsigned 64-bit integers, so the result saturates at zero
on subtraction and at `u64::MAX` on addition; the bucket value will never
go negative or wrap around.

Use [increment](increment.md) when you only need to add.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'inflight',
  num_buckets = 5,
  bucket_size = '1m',
}
series:delta(1) -- a job started
series:delta(-1) -- a job finished
```
