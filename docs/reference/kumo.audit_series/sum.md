---
tags:
  - audit_series
---

# audit_series.sum

```lua
audit:sum()
```

{{since('dev')}}

Returns the rolling total across all configured windows.

Use this when you want the full series total.
Use [audit_series.sum_over](sum_over.md) to sum a shorter span.

Error is generated if audit series name is not yet defined.

```lua
print('Total Value: ', audit:sum())
```