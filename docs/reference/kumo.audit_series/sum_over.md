---
tags:
  - audit_series
---

# audit_series.sum_over

```lua
audit:sum_over(duration)
```

{{since('dev')}}

Returns the rolling total over the requested time span.

* `duration`: the span to sum over.

The duration is rounded up to full buckets and capped by the configured window count.

Error is generated if audit series name is not yet defined.

```lua
local audit = kumo.audit_series.define('test.audit', 5, '1m')
print('last 5m:', audit:sum_over('5m'))
```