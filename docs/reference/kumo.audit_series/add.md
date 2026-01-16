---
tags:
  - audit_series
---

# audit_series.add

```lua
kumo.audit_series.add(name, options)
```

{{since('dev')}}

Increment or decrement the counter in current bucket.

`key` : key name to track counters for

`count` : value to increment or use negative number to decrement

Returned value represents the value in the current bucket. Use the get function to obtain the total of all buckets.

Error is generated if audit series name is not yet defined.

```
  local current_counter = kumo.audit_series.add(
    'failure_count',
    { key = 'element_name',
      count = 1
    }
  )
```
