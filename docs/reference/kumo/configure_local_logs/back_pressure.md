---
tags:
 - logging
---

# back_pressure

Maximum number of outstanding items to be logged before
the submission will block; helps to avoid runaway issues
spiralling out of control.

```lua
kumo.configure_local_logs {
  -- ..
  back_pressure = 128000,
}
```


