# kumo.time.from_unix_timestamp

{{since('2025.12.02-67ee9e96')}}

```lua
local time = kumo.time.from_unix_timestamp(UNIX_TIMESTAMP)
```

Constructs a new [Time](Time.md) object representing the specified unix timestamp.

The timestamp can be either an integer number of seconds, or a fractional
number of seconds since the unix epoch.


