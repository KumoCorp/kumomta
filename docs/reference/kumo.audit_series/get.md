---
tags:
  - audit_series
---

# audit_series.get

```lua
kumo.audit_series.get(name, key)
```

{{since('dev')}}

Returns the total counter of all windows for given name.

`name`: audit series name registered through define function
`key` : key name to track counters for

Error is generated if audit series name is not yet defined.

```lua
  local counter = kumo.audit_series.get(
    'failure_count',
    'element_name'
  )
```
