# TimeDelta Object

{{since('dev')}}

The `TimeDelta` object represents a time interval.

## Metamethods

The following metamethod are implemented on `TimeDelta` objects:

 * `__tostring` - returns a human readable duration string, the same as the `human` field described below
 * `__eq` - compares two `TimeDelta` objects for equality
 * `+` - you may add a `TimeDelta` to a `TimeDelta` to produce a new `TimeDelta`
 * `-` - you may subtract a `TimeDelta` from a `TimeDelta` to produce a new `TimeDelta`

## Fields

The following fields epose information about the underlying `TimeDelta`.
Fields are accessed using dot notation, like `delta.seconds`.

 * `seconds` - returns the TimeDelta expressed as a signed number of seconds (including fractional seconds)
 * `nanoseconds` - returns the TimeDelta expressed as a signed integer number of nanoseconds.
 * `milliseconds` - returns the TimeDelta expressed as a signed integer number of milliseconds.
 * `microseconds` - returns the TimeDelta expressed as a signed integer number of microseconds.
 * `human` - returns the TimeDelta expressed as a human readable string, such
   as `5m` for a five minute duration.
