---
tags:
 - logging
 - debugging
---

# kumo.log_debug

```lua
kumo.log_debug(ARGS)
```

{{since('2025.03.19-1d3f1f67')}}

Logs the series of `ARGS` to the diagnostic log at `DEBUG` level.
This works similarly to the `print` function except that it is routed
via the diagnostic logging system, which might be set to filter out
the event via the [set_diagnostic_log_filter](set_diagnostic_log_filter.md).

!!! note
    DEBUG is not included in the default diagnostic filter.
    You will need to explicitly add `lua=DEBUG` to your diagnostic filter
    in order to see `kumo.log_debug` diagnostics.

The purpose of this function is to log meaningful information from your
policy scripts for diagnostic purposes.

```lua
-- I am a file named unix.lua
-- The next line is line number 3
kumo.log_debug('Logging something', true, false, 42, { 1, 2, 3 })
```

Will produce something like this in your diagnostic log, but only if you have
enabled `lua=DEBUG` in your diagnostic filter:

```
2025-03-07T00:08:20.071516Z DEBUG  main lua: ./unix.lua:3: Logging something true false 42 table: 0x7199e11bda40
```

The `main` string there is the thread name. You can see that the calling source
file and line number are automatically included in the diagnostic record.  The
arguments are converted to strings via the equivalent of the lua `tostring()`
function and output as part of the diagnostic.

