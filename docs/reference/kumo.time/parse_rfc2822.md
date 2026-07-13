# kumo.time.parse_rfc2822

{{since('2025.12.02-67ee9e96')}}

```lua
local time = kumo.time.parse_rfc2822(TIMESTAMP)
```

Parses a timestamp in [RFC 2822](https://datatracker.ietf.org/doc/html/rfc2822)
format, and returns a [Time](Time.md) object.

## Example

```lua
local t = kumo.time.parse_rfc2822 'Sun, 2 Jan 2000 03:04:05 +0000'
```

