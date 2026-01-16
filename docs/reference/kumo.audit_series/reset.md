---
tags:
  - audit_series
---

# audit_series.reset

```lua
kumo.audit_series.reset(name, options)
```

{{since('dev')}}

Sets all counter to 0 for given audit_series name.

Error is generated if audit series name is not yet defined.

```
  kumo.audit_series.reset(
    'failure_counter',
    { key = 'entity1' }
  )
```
