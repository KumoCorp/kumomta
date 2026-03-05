---
tags:
  - audit_series
---

# audit_series.add

```lua
kumo.audit_series.add(name, key, count)
```

{{since('dev')}}

Increment or decrement the counter in current window.

`name`: audit series name registered through define function
`key` : key name of the window
`count` : value to increment or use negative number to decrement

Returned value represents the value in the current window. Use the get function to obtain the total of all windows.

Error is generated if audit series name is not yet defined.

```lua
  local current_counter = kumo.audit_series.add(
    'failure_count',
    'element_name',
    1
  )
```
