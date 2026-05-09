---
tags:
 - counter_series
---

# Module `kumo.counter_series`

{{since('dev')}}

The `kumo.counter_series` module exposes named, in-memory rolling counters
backed by a fixed-size ring of time buckets. It is intended for short-term
bookkeeping of event rates within the current process — for example, counting
recent successful or failed deliveries to a destination, or tracking how many
times a policy decision was taken in the last few minutes.

Each named series is created via [counter_series.define](define.md). The
returned handle is a userdata object with the methods listed below.

## Properties

* **In-memory only.** Values are not persisted across process restarts.
* **Per-process.** Values are not shared between kumomta nodes. If you need
  cross-node visibility or persistence, use an external store such as Redis.
* **Cached by name.** Repeated calls to `define` with the same `name` and
  shape (`num_buckets`, `bucket_size`) return the same underlying series, so
  it is safe to call `define` from a hot path.
* **Saturating.** Bucket values are unsigned 64-bit integers; underflow
  saturates at zero and overflow saturates at `u64::MAX`.

## Available Methods { data-search-exclude }
