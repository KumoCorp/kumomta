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

local now = kumo.time.now()
-- Note: not using utils.assert_ne here due to weirdness with
-- mlua and getmetatable on the Time type
assert(now ~= t)
kumo.time.sleep(0.1)
local elapsed = now.elapsed.seconds
print('elapsed', elapsed, now.elapsed)
assert(now.elapsed.seconds >= 0.1)

local rfc2822 = kumo.time.parse_rfc2822(t.rfc2822)
utils.assert_eq(t, rfc2822)

local rfc3339 = kumo.time.parse_rfc3339(t.rfc3339)
utils.assert_eq(t, rfc3339)

local unix = kumo.time.from_unix_timestamp(t.unix_timestamp)
utils.assert_eq(t, unix)

-- Verify that a technically invalid format string is processed
-- leniently, without panic.
local ut = t:format '%H:%M:%S%'
utils.assert_eq(ut, '03:04:05%')

local unix_epoch = kumo.time.from_unix_timestamp(0)
local half_second = kumo.time.from_unix_timestamp(0.5)
utils.assert_eq(tostring(half_second), '1970-01-01 00:00:00.500 UTC')

-- Verify Time Add/Sub metamethods
local delta = half_second - unix_epoch
utils.assert_eq(delta.seconds, 0.5)
utils.assert_eq(delta.human, '500ms')

local one_sec = half_second + delta
utils.assert_eq(tostring(one_sec), '1970-01-01 00:00:01 UTC')

local epoch_again = half_second - kumo.time.parse_duration(0.5)
utils.assert_eq(tostring(epoch_again), '1970-01-01 00:00:00 UTC')

-- Verify TimeDelta Add/Sub metamethods
local one_and_a_half_sec_delta = delta + delta + delta
utils.assert_eq(one_and_a_half_sec_delta.seconds, 1.5)

local one_sec_delta = one_and_a_half_sec_delta - delta
utils.assert_eq(one_sec_delta.seconds, 1.0)
utils.assert_eq(one_sec_delta.nanoseconds, 1000000000)
utils.assert_eq(one_sec_delta.microseconds, 1000000)
utils.assert_eq(one_sec_delta.milliseconds, 1000)
utils.assert_eq(one_sec_delta.human, '1s')

-- duration parsing
local d = kumo.time.parse_duration(2.5)
utils.assert_eq(d.seconds, 2.5)

local d = kumo.time.parse_duration(2)
utils.assert_eq(d.seconds, 2)

utils.assert_eq(kumo.time.parse_duration(2), kumo.time.parse_duration '2s')

local d = kumo.time.parse_duration '5 mins'
utils.assert_eq(d.seconds, 300)
utils.assert_eq(d.human, '5m')
utils.assert_eq(tostring(d), '5m')
