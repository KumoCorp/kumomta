---
status: deprecated
---

# kumo.sleep

```lua
kumo.sleep(SECONDS)
```

!!! warning
    This function has moved to the [kumo.time](../kumo.time/index.md) module and
    will be removed in a future release.
    {{since('2025.01.23-7273d2bc', inline=True)}}

{{since('2024.06.10-84e84b89')}}

Sleeps the current task for the specified number of seconds.
The value can be either an integer or a floating point value,
the latter can be used to specify fractional duration values.
