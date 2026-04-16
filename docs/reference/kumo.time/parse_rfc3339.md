# kumo.time.parse_rfc3339

{{since('2025.12.02-67ee9e96')}}

```lua
local time = kumo.time.parse_rfc3339(TIMESTAMP)
```

Parses a timestamp in [RFC 3339](https://datatracker.ietf.org/doc/html/rfc3339)
format, and returns a [Time](Time.md) object.

## Example

```lua
local t = kumo.time.parse_rfc3339 '2000-01-02T03:04:05+00:00'
```
