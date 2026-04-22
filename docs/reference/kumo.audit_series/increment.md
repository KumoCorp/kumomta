---
tags:
  - audit_series
---

# audit_series.increment

```lua
audit:increment(value)
```

{{since('dev')}}

Adds `value` to the current window.

* `value`: unsigned integer amount to add.

To subtract, use [audit_series.delta](delta.md) with a negative value.

Error is generated if audit series name is not yet defined.

```lua
audit:increment(1)
```