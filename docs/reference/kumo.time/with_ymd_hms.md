# kumo.time.with_ymd_hms

{{since('2025.12.02-67ee9e96')}}

```lua
local time = kumo.time.with_ymd_hms(YEAR, MONTH, DAY, HOUR, MINUTE, SECOND)
```

Constructs a new [Time](Time.md) object representing the UTC date and time specified.

## Example

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
local t = kumo.time.with_ymd_hms(2000, 01, 02, 03, 04, 05)
utils.assert_eq(tostring(t), '2000-01-02 03:04:05 UTC')
utils.assert_eq(t.year, 2000)
utils.assert_eq(t.month, 1)
utils.assert_eq(t.day, 2)
utils.assert_eq(t.hour, 3)
utils.assert_eq(t.minute, 4)
utils.assert_eq(t.second, 5)
utils.assert_eq(t.unix_timestamp, 946782245)
utils.assert_eq(t.rfc2822, 'Sun, 2 Jan 2000 03:04:05 +0000')
utils.assert_eq(t.rfc3339, '2000-01-02T03:04:05+00:00')
```
