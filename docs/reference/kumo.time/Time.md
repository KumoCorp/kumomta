# Time Object Type

{{since('2025.12.02-67ee9e96')}}

The `Time` object type represents a date and time. It internally stores the time as
[UTC](https://en.wikipedia.org/wiki/UTC%2B00:00), even if it was produced by
parsing time from some other time zone.

You do not create a `Time` object directly, but instead using one of the constructing functions:

 * [kumo.time.from_unix_timestamp](from_unix_timestamp.md)
 * [kumo.time.now](now.md)
 * [kumo.time.parse_rfc2822](parse_rfc2822.md)
 * [kumo.time.parse_rfc3339](parse_rfc3339.md)
 * [kumo.time.with_ymd_hms](with_ymd_hms.md)

or though metamethods of `Time` (see below).

## Metamethods

The following metamethods are implemented on `Time` objects:

 * `tostring(some_time)` - returns a human readable representation of the time like `1970-01-01 00:00:01 UTC`.
 * `time1 == time2` - compares two `Time` objects for equality
 * `some_time + time_delta` - addition. You may add a [TimeDelta](TimeDelta.md) to a `Time` to produce a new
   `Time` offset by the added delta.
 * `-` - subtraction:
    * `time1 - time_delta` - You may subtract a [TimeDelta](TimeDelta.md) from
      a `Time` to produce a new `Time` offset by the added delta.
    * `time1 - time2` - You may subtract a `Time` from a `Time` to produce a
      [TimeDelta](TimeDelta.md) representing the difference between the two
      times

## Fields

The following fields expose various properties of the underlying `Time` object.
Fields are accessed using dot notation, like `time.rfc2822`.

 * `year` - the year number portion of the ISO 8601 calendar date.
 * `month` - the month number of the calendar date, starting with `1` for January.
 * `day` - the day of the month of the calendar date, starting from `1`.
 * `hour` - the hour number, from `0` to `23`.
 * `minute` - the minute number, from `0` to `59`.
 * `second` - the second number, from `0` to `59`.
 * `unix_timestamp` - the number of non-leap seconds since January 1, 1970
   0:00:00 UTC (aka "UNIX timestamp").
 * `unix_timestamp_millis` - the number of non-leap milliseconds since January
   1, 1970 0:00:00 UTC.
 * `rfc2822` - the time formatted according to RFC 2822
 * `rfc3339` - the time formatted according to RFC 3339
 * `elapsed` - the [TimeDelta](TimeDelta.md) corresponding to the difference
   between *this* `Time` object and the time at which the `elapsed` field is
   evaluated.

```lua
local now = kumo.time.now()
print(now.year)
print(now.unix_timestamp)
```

## Methods

The following methods are implemented on `Time` objects. Methods are accessed
using colon notation, like `time:format()`.

### Time:format

```lua
local now = kumo.time.now()
local string = now:format '%H:%M:%S'
```

Formats the time object using the [strftime
syntax](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)
supported by the Rust chrono crate.

Parsing of the format string is lenient, but it is still possible that
for invalid format strings to raise errors.



