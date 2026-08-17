---
tags:
  - counter_series
---

# counter_series.define

```lua
kumo.counter_series.define {
  name = 'deliveries.ok',
  num_buckets = 5,
  bucket_size = '1m',
  initial_value = 0,
}
```

{{since('2026.05.12-a6845223')}}

Creates or returns a named rolling counter series and returns a handle to
it. Takes a single table argument with the following fields:

## Parameters

* `name` — unique series name. Series are cached by name across calls.
* `num_buckets` — integer count of time buckets to maintain. Must be `>= 1`.
* `bucket_size` — duration of each bucket. Accepts an integer number of
  seconds, or a duration string such as `"5s"`, `"1m"`, `"500ms"`.
  Sub-second values are rounded **up** to the next whole second, so
  `"500ms"` becomes a 1 second bucket. The minimum effective bucket size is
  therefore 1 second.
* `initial_value` — optional `u64` to seed the current bucket with when the
  series is first created (or replaced — see below). Defaults to `0`.

The total retention span of the series is `num_buckets * bucket_size`.

## Caching and re-definition

Repeated calls to `define` with the same `name` behave as follows:

* If `num_buckets` and the resolved `bucket_size` (in whole seconds) match the
  cached series, the existing series is returned and `initial_value` is
  ignored. Existing counts are preserved.
* If either `num_buckets` or `bucket_size` differ from the cached series,
  the cached series is **replaced** with a fresh one using the new
  parameters and `initial_value`. Previously recorded counts are discarded.

This makes `define` safe to call repeatedly from policy hot paths, while
also allowing a configuration reload to reshape a series.

## Example

```lua
local series = kumo.counter_series.define {
  name = 'deliveries.ok',
  num_buckets = 5,
  bucket_size = '1m',
}
series:increment(1)
```
