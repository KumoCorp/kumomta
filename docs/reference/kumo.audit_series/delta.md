---
tags:
  - audit_series
---

# audit_series.delta

```lua
audit:delta(value)
```

{{since('dev')}}

Adjusts the current window by a signed amount.

* `value`: signed integer delta.

Positive values increase; negative values decrease.
The value saturates at `0` and never goes negative.

Error is generated if audit series name is not yet defined.

```lua
audit:delta(2)
audit:delta(-1)
```