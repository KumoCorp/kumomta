---
tags:
  - audit_series
---

# audit_series.reset

```lua
kumo.audit_series.reset(name, key)
```

{{since('dev')}}

Sets all counter to 0 for given audit_series name.

`name`: audit series name registered through define function
`key` : key name to track counters for

Error is generated if audit series name is not yet defined.

```lua
  kumo.audit_series.reset(
    'failure_counter',
    'entity1'
  )
```
