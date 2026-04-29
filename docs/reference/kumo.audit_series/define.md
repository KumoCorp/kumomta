---
tags:
  - audit_series
---

# audit_series.define

```lua
kumo.audit_series.define(name, window_count, window_duration, initial_value)
```

{{since('dev')}}

Creates and returns a named rolling counter series.

* `name`: unique series name. Calling `define` again with the same name returns the existing series; other arguments are ignored.
* `window_count`: window count to maintain.
* `window_duration` : duration of each window. Accepts seconds or a duration string like `"5s"`.
* `initial_value` : optional value to initialize the series. Omit to initialize as zero.

Total retention span is `window_count * window_duration`.
Values are stored as `u64`.

```lua
local audit = kumo.audit_series.define('test.audit', 5, '1m')
audit:increment(1)
```