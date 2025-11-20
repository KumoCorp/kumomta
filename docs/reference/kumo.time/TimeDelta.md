# TimeDelta Object Type

{{since('dev')}}

The `TimeDelta` object type represents a time interval.

You do not create a `TimeDelta` object directly, but instead using one of the
constructing functions:

 * [kumo.time.parse_duration](parse_duration.md)

or though metamethods of `TimeDelta` (see below) or [Time](Time.md#metamethod).

## Metamethods

The following metamethod are implemented on `TimeDelta` objects:

 * `tostring(time_delta)` - returns a human readable duration string, the same as the `human` field described below
 * `delta1 == delta2` - compares two `TimeDelta` objects for equality
 * `delta1 + delta2` - you may add a `TimeDelta` to a `TimeDelta` to produce a new `TimeDelta`
 * `delta1 - delta2` - you may subtract a `TimeDelta` from a `TimeDelta` to produce a new `TimeDelta`

## Fields

The following fields epose information about the underlying `TimeDelta`.
Fields are accessed using dot notation, like `delta.seconds`.

 * `seconds` - returns the TimeDelta expressed as a signed number of seconds (including fractional seconds)
 * `nanoseconds` - returns the TimeDelta expressed as a signed integer number of nanoseconds.
 * `milliseconds` - returns the TimeDelta expressed as a signed integer number of milliseconds.
 * `microseconds` - returns the TimeDelta expressed as a signed integer number of microseconds.
 * `human` - returns the TimeDelta expressed as a human readable string, such
   as `5m` for a five minute duration.

```lua
local delta1 = kumo.time.parse_duration(20)
local delta2 = kumo.time.parse_duration '10 seconds'

assert((delta2 - delta1).seconds == 10)
```
