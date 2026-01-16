---
tags:
  - audit_series
---

# audit_series.define

```lua
kumo.audit_series.get(name, options)
```

{{since('dev')}}

Returns the total counter of all buckets for given name.

Error is generated if audit series name is not yet defined.

```
  local counter = kumo.audit_series.get(
    'failure_count',
    { key = 'element_name' }
  )
```
