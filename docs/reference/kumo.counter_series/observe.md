---
tags:
  - counter_series
---

# counter_series:observe

```lua
series:observe(value)
```

{{since('dev')}}

Sets the current bucket of the series to `value`, replacing any prior value
recorded for the current time window.

* `value` — unsigned integer to record. Must be `>= 0`.

`observe` is intended for tracking gauge-like measurements where each call
produces a fresh reading (for example, the size of a queue sampled
periodically), as opposed to [increment](increment.md) which accumulates.

Older buckets are not affected; they continue to age out as time elapses.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'queue.depth',
  num_buckets = 6,
  bucket_size = '10s',
}
series:observe(current_queue_depth())
```
