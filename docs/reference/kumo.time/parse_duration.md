# kumo.time.parse_duration

{{since('dev')}}

```lua
local time = kumo.time.parse_duration(DURATION)
```

Parses a duration to create a [TimeDelta](TimeDelta.md) object.

`DURATION` can be:

 * A signed integer number of seconds
 * A signed floating point number of seconds
 * A duration string like `5 minutes`

## Example

```lua
local delta = kumo.time.parse_duration '5m'
```


